//! HEIC and HEIF (and Sony's `.hif` name for HEIF) through the operating
//! system codec: Windows Imaging
//! Component on Windows (needs the HEIF and HEVC extensions from the
//! Microsoft Store) and ImageIO on macOS. Linux has no provider.
//!
//! With `DecodeOptions::target_size` on Windows, the thumbnail the file
//! carries (Sony stores 1616 x 1080) is decoded instead of the image when
//! it covers the target and has the image's aspect ratio: about 60 to 110
//! ms instead of a second for a 60 megapixel file.
//!
//! This is the only module with `unsafe` code besides the SIMD kernels:
//! COM and CoreFoundation calls. Each block states why it is sound.

#![allow(unsafe_code)]

use crate::util::{self, check_size, corrupt};
use crate::{icc, jpeg, orient};
use pandite_core::{
    CancelToken, Content, ContentKind, DecodeOptions, DecoderProvider, ImageBuf, ImageFacts,
    ImageMeta, Orientation, PixelFormat, Result, Source,
};

const FORMAT: &str = "heic";

/// Decodes HEIC and HEIF files with the platform codec.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeicDecoder;

/// True for HEIF brands that are not AVIF.
fn is_heif(header: &[u8]) -> bool {
    let mut heif = false;
    for b in util::ftyp_brands(header) {
        match &b {
            b"avif" | b"avis" => return false,
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"hevm" | b"hevs"
            | b"mif1" | b"msf1" | b"mif2" => heif = true,
            _ => {}
        }
    }
    heif
}

impl DecoderProvider for HeicDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "HEIF image (system codec)"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["heic", "heif", "hif"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        is_heif(header)
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let (w, h, orientation) = os::probe(source.bytes())?;
        let (width, height) = jpeg::oriented(w, h, orientation);
        Ok(ImageMeta {
            width,
            height,
            format: FORMAT,
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: false,
            orientation,
        })
    }

    fn reduced_decode(&self, _source: &Source) -> bool {
        // Windows Imaging Component hands out the file's thumbnail.
        cfg!(windows)
    }

    fn facts(&self, source: &Source) -> Result<ImageFacts> {
        // The container reader lives with the AVIF decoder; the system
        // codec gives no header facts.
        #[cfg(feature = "avif")]
        let facts = util::guard(FORMAT, || Ok(crate::avif::heif_facts(source.bytes())))?;
        #[cfg(not(feature = "avif"))]
        let facts = {
            let _ = source;
            ImageFacts::named("HEIF")
        };
        Ok(facts)
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        let (image, orientation) = util::guard(FORMAT, || {
            os::decode(source.bytes(), options.target_size, &options.cancel)
        })?;
        Ok(Content::Still(orient::apply(image, orientation)?))
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
/// True when a thumbnail of `thumb` pixels stands in for an image of
/// `full` pixels at `target` (stored axes): smaller than the image, the
/// same aspect ratio within 2 percent, and reaching the target on one
/// axis.
fn thumbnail_fits(thumb: (u32, u32), full: (u32, u32), target: (u32, u32)) -> bool {
    let (tw, th) = thumb;
    if tw == 0 || th == 0 || full.0 == 0 || full.1 == 0 {
        return false;
    }
    let smaller = u64::from(tw) * u64::from(th) < u64::from(full.0) * u64::from(full.1);
    let aspect = (f64::from(tw) / f64::from(th)) / (f64::from(full.0) / f64::from(full.1));
    smaller && (aspect - 1.0).abs() <= 0.02 && (tw >= target.0 || th >= target.1)
}

/// Finish a decoded RGBA8 buffer: color management, then the image.
fn finish(w: u32, h: u32, mut rgba: Vec<u8>, profile: Option<&[u8]>) -> Result<ImageBuf> {
    icc::to_srgb_rgba8(profile, &mut rgba);
    util::rgba8_image(w, h, rgba)
}

#[cfg(windows)]
mod os {
    use super::*;
    use pandite_core::Error;
    use windows::core::w;
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICBitmapFrameDecode,
        IWICBitmapSource, IWICColorContext, IWICImagingFactory, WICBitmapDitherTypeNone,
        WICBitmapPaletteTypeCustom, WICColorContextProfile, WICDecodeMetadataCacheOnDemand,
    };
    use windows::Win32::System::Com::StructuredStorage::{
        PropVariantClear, PropVariantToUInt16, PROPVARIANT,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };

    fn backend(what: &str, e: windows::core::Error) -> Error {
        Error::Backend(format!("{FORMAT}: {what}: {e}"))
    }

    /// Balances a successful `CoInitializeEx` on this thread.
    struct ComScope(bool);

    impl ComScope {
        fn enter() -> Self {
            // SAFETY: CoInitializeEx has no preconditions. A thread already
            // in a single-threaded apartment returns RPC_E_CHANGED_MODE; COM
            // is usable then and must not be uninitialized by us.
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            ComScope(hr.is_ok())
        }
    }

    impl Drop for ComScope {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: paired with the successful CoInitializeEx in
                // `enter` on the same thread; all COM objects created in the
                // scope are dropped before this guard (declared first).
                unsafe { CoUninitialize() };
            }
        }
    }

    /// Open the first frame. The returned objects borrow `data` through
    /// the WIC stream, so they must not outlive it; callers keep both in
    /// one function scope.
    fn open(data: &[u8]) -> Result<(IWICImagingFactory, IWICBitmapFrameDecode)> {
        // SAFETY: plain COM calls on interfaces we own. The stream reads
        // from `data`, which the caller keeps alive while the frame lives.
        unsafe {
            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| backend("no imaging factory", e))?;
            let stream = factory.CreateStream().map_err(|e| backend("stream", e))?;
            stream
                .InitializeFromMemory(data)
                .map_err(|e| backend("stream", e))?;
            let decoder = factory
                .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
                .map_err(|e| {
                    Error::Unsupported(format!(
                        "{FORMAT}: no system codec could open the file ({e}); install the HEIF \
                         and HEVC extensions"
                    ))
                })?;
            let frame = decoder.GetFrame(0).map_err(|e| corrupt(FORMAT, e))?;
            Ok((factory, frame))
        }
    }

    fn size(frame: &IWICBitmapFrameDecode) -> Result<(u32, u32)> {
        let (mut w, mut h) = (0u32, 0u32);
        // SAFETY: GetSize writes two u32 values through valid pointers.
        unsafe { frame.GetSize(&mut w, &mut h) }.map_err(|e| corrupt(FORMAT, e))?;
        Ok((w, h))
    }

    fn orientation(frame: &IWICBitmapFrameDecode) -> Orientation {
        // SAFETY: the query reader belongs to the frame; the PROPVARIANT is
        // zero initialized, filled by WIC and cleared before it drops.
        unsafe {
            let Ok(reader) = frame.GetMetadataQueryReader() else {
                return Orientation::Normal;
            };
            for name in [w!("System.Photo.Orientation"), w!("/heifProps/{ushort=1}")] {
                let mut value = PROPVARIANT::default();
                if reader.GetMetadataByName(name, &mut value).is_ok() {
                    let v = PropVariantToUInt16(&value).unwrap_or(1);
                    let _ = PropVariantClear(&mut value);
                    return Orientation::from_exif(v);
                }
            }
        }
        Orientation::Normal
    }

    fn profile(factory: &IWICImagingFactory, frame: &IWICBitmapFrameDecode) -> Option<Vec<u8>> {
        // SAFETY: GetColorContexts first reports the count, then fills
        // contexts we created; GetProfileBytes writes into a buffer of the
        // size it reported.
        unsafe {
            let mut count = 0u32;
            frame.GetColorContexts(&mut [], &mut count).ok()?;
            if count == 0 {
                return None;
            }
            let mut contexts: Vec<Option<IWICColorContext>> = Vec::new();
            for _ in 0..count.min(8) {
                contexts.push(Some(factory.CreateColorContext().ok()?));
            }
            let mut actual = 0u32;
            frame.GetColorContexts(&mut contexts, &mut actual).ok()?;
            for ctx in contexts.iter().flatten().take(actual as usize) {
                if ctx.GetType().ok()? != WICColorContextProfile {
                    continue;
                }
                let mut len = 0u32;
                ctx.GetProfileBytes(&mut [], &mut len).ok()?;
                let mut bytes = vec![0u8; len as usize];
                ctx.GetProfileBytes(&mut bytes, &mut len).ok()?;
                return Some(bytes);
            }
        }
        None
    }

    pub(super) fn probe(data: &[u8]) -> Result<(u32, u32, Orientation)> {
        let _com = ComScope::enter();
        let (_factory, frame) = open(data)?;
        let (w, h) = size(&frame)?;
        Ok((w, h, orientation(&frame)))
    }

    /// The codec's native pixel format of the first frame: its GUID, bits
    /// per pixel and channel count. For tests that report bit depths.
    #[cfg(test)]
    pub(super) fn source_format(data: &[u8]) -> Result<(String, u32, u32)> {
        use windows::core::Interface;
        use windows::Win32::Graphics::Imaging::IWICPixelFormatInfo;
        let _com = ComScope::enter();
        let (factory, frame) = open(data)?;
        // SAFETY: plain COM calls on interfaces owned by this scope.
        unsafe {
            let guid = frame
                .GetPixelFormat()
                .map_err(|e| backend("pixel format", e))?;
            let info: IWICPixelFormatInfo = factory
                .CreateComponentInfo(&guid)
                .map_err(|e| backend("component info", e))?
                .cast()
                .map_err(|e| backend("pixel format info", e))?;
            let bpp = info.GetBitsPerPixel().map_err(|e| backend("bits", e))?;
            let channels = info.GetChannelCount().map_err(|e| backend("channels", e))?;
            Ok((format!("{guid:?}"), bpp, channels))
        }
    }

    /// The size of a bitmap source.
    fn source_size(source: &IWICBitmapSource) -> Result<(u32, u32)> {
        let (mut w, mut h) = (0u32, 0u32);
        // SAFETY: GetSize writes two u32 values through valid pointers.
        unsafe { source.GetSize(&mut w, &mut h) }.map_err(|e| corrupt(FORMAT, e))?;
        Ok((w, h))
    }

    /// Convert `source` to RGBA8 in a buffer of its size.
    fn pixels(
        factory: &IWICImagingFactory,
        source: &IWICBitmapSource,
    ) -> Result<(u32, u32, Vec<u8>)> {
        let (w, h) = source_size(source)?;
        let len = check_size(w, h, PixelFormat::Rgba8)?;
        let mut rgba = util::alloc_touched(len, w, h)?;
        // SAFETY: the converter reads from a source we own and writes
        // exactly `stride * h` bytes into `rgba`, which has that length.
        unsafe {
            let converter = factory
                .CreateFormatConverter()
                .map_err(|e| backend("converter", e))?;
            converter
                .Initialize(
                    source,
                    &GUID_WICPixelFormat32bppRGBA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )
                .map_err(|e| corrupt(FORMAT, e))?;
            converter
                .CopyPixels(std::ptr::null(), w * 4, &mut rgba)
                .map_err(|e| corrupt(FORMAT, e))?;
        }
        Ok((w, h, rgba))
    }

    /// Decode the image, or its thumbnail when `target` (display axes)
    /// asks for a smaller image and the thumbnail covers it.
    pub(super) fn decode(
        data: &[u8],
        target: Option<(u32, u32)>,
        cancel: &CancelToken,
    ) -> Result<(ImageBuf, Orientation)> {
        let _com = ComScope::enter();
        let (factory, frame) = open(data)?;
        let full = size(&frame)?;
        let orientation = orientation(&frame);
        cancel.check()?;
        let icc = profile(&factory, &frame);
        if let Some((tw, th)) = target {
            // The thumbnail is stored like the image: compare on stored axes.
            let stored = jpeg::oriented(tw, th, orientation);
            // SAFETY: a plain COM call on the frame we own.
            let thumb = unsafe { frame.GetThumbnail() };
            if let Ok(thumb) = thumb {
                let fits = source_size(&thumb).is_ok_and(|t| thumbnail_fits(t, full, stored));
                if fits {
                    if let Ok((w, h, rgba)) = pixels(&factory, &thumb) {
                        return Ok((finish(w, h, rgba, icc.as_deref())?, orientation));
                    }
                }
            }
        }
        let source: IWICBitmapSource =
            windows::core::Interface::cast(&frame).map_err(|e| backend("bitmap source", e))?;
        let (w, h, rgba) = pixels(&factory, &source)?;
        Ok((finish(w, h, rgba, icc.as_deref())?, orientation))
    }
}

#[cfg(target_os = "macos")]
mod os {
    use super::*;
    use std::ffi::c_void;

    type CFTypeRef = *const c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDataCreateWithBytesNoCopy(
            allocator: CFTypeRef,
            bytes: *const u8,
            length: isize,
            deallocator: CFTypeRef,
        ) -> CFTypeRef;
        fn CFRelease(cf: CFTypeRef);
        fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        fn CFNumberGetValue(number: CFTypeRef, the_type: isize, value: *mut c_void) -> u8;
        static kCFAllocatorNull: CFTypeRef;
    }

    #[link(name = "ImageIO", kind = "framework")]
    extern "C" {
        fn CGImageSourceCreateWithData(data: CFTypeRef, options: CFTypeRef) -> CFTypeRef;
        fn CGImageSourceGetCount(source: CFTypeRef) -> usize;
        fn CGImageSourceCreateImageAtIndex(
            source: CFTypeRef,
            index: usize,
            options: CFTypeRef,
        ) -> CFTypeRef;
        fn CGImageSourceCopyPropertiesAtIndex(
            source: CFTypeRef,
            index: usize,
            options: CFTypeRef,
        ) -> CFTypeRef;
        static kCGImagePropertyOrientation: CFTypeRef;
        static kCGImagePropertyPixelWidth: CFTypeRef;
        static kCGImagePropertyPixelHeight: CFTypeRef;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGImageGetWidth(image: CFTypeRef) -> usize;
        fn CGImageGetHeight(image: CFTypeRef) -> usize;
        fn CGColorSpaceCreateWithName(name: CFTypeRef) -> CFTypeRef;
        fn CGColorSpaceRelease(space: CFTypeRef);
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits_per_component: usize,
            bytes_per_row: usize,
            space: CFTypeRef,
            bitmap_info: u32,
        ) -> CFTypeRef;
        fn CGContextDrawImage(context: CFTypeRef, rect: CGRect, image: CFTypeRef);
        fn CGContextRelease(context: CFTypeRef);
        fn CGImageRelease(image: CFTypeRef);
        static kCGColorSpaceSRGB: CFTypeRef;
    }

    /// kCFNumberSInt64Type.
    const CF_NUMBER_SINT64: isize = 4;
    /// kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big.
    const RGBA_PREMULTIPLIED: u32 = 1 | (4 << 12);

    /// Owns one CoreFoundation reference.
    struct Cf(CFTypeRef);

    impl Drop for Cf {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the pointer came from a Create or Copy function,
                // so we own exactly one reference.
                unsafe { CFRelease(self.0) };
            }
        }
    }

    /// An image source over `data`. The source must not outlive `data`:
    /// the CFData wraps the slice without copying.
    fn source(data: &[u8]) -> Result<(Cf, Cf)> {
        let len = isize::try_from(data.len()).map_err(|_| corrupt(FORMAT, "file too large"))?;
        // SAFETY: CFDataCreateWithBytesNoCopy with kCFAllocatorNull wraps
        // the slice without taking ownership; callers drop both returned
        // objects before `data` goes away.
        unsafe {
            let cfdata = Cf(CFDataCreateWithBytesNoCopy(
                std::ptr::null(),
                data.as_ptr(),
                len,
                kCFAllocatorNull,
            ));
            if cfdata.0.is_null() {
                return Err(corrupt(FORMAT, "cannot wrap data"));
            }
            let src = Cf(CGImageSourceCreateWithData(cfdata.0, std::ptr::null()));
            if src.0.is_null() || CGImageSourceGetCount(src.0) == 0 {
                return Err(pandite_core::Error::Unsupported(format!(
                    "{FORMAT}: ImageIO cannot open the file"
                )));
            }
            // Drop order: the source first, then the data it reads.
            Ok((src, cfdata))
        }
    }

    fn number(dict: CFTypeRef, key: CFTypeRef) -> Option<i64> {
        // SAFETY: `dict` is a live CFDictionary; the value, when present,
        // is a CFNumber borrowed from it (Get rule, no release).
        unsafe {
            let v = CFDictionaryGetValue(dict, key);
            if v.is_null() {
                return None;
            }
            let mut out: i64 = 0;
            (CFNumberGetValue(v, CF_NUMBER_SINT64, &mut out as *mut i64 as *mut c_void) != 0)
                .then_some(out)
        }
    }

    fn properties(src: &Cf) -> (Option<u32>, Option<u32>, Orientation) {
        // SAFETY: the properties dictionary is a Copy result we release.
        unsafe {
            let props = Cf(CGImageSourceCopyPropertiesAtIndex(
                src.0,
                0,
                std::ptr::null(),
            ));
            if props.0.is_null() {
                return (None, None, Orientation::Normal);
            }
            let w = number(props.0, kCGImagePropertyPixelWidth).and_then(|v| u32::try_from(v).ok());
            let h =
                number(props.0, kCGImagePropertyPixelHeight).and_then(|v| u32::try_from(v).ok());
            let o = number(props.0, kCGImagePropertyOrientation)
                .map(|v| Orientation::from_exif(v as u16))
                .unwrap_or_default();
            (w, h, o)
        }
    }

    pub(super) fn probe(data: &[u8]) -> Result<(u32, u32, Orientation)> {
        let (src, _data) = source(data)?;
        match properties(&src) {
            (Some(w), Some(h), o) => Ok((w, h, o)),
            _ => Err(corrupt(FORMAT, "no image size")),
        }
    }

    /// Decode the image. ImageIO's thumbnails are not used: `_target` is
    /// ignored and the full image is decoded.
    pub(super) fn decode(
        data: &[u8],
        _target: Option<(u32, u32)>,
        cancel: &CancelToken,
    ) -> Result<(ImageBuf, Orientation)> {
        let (src, _data) = source(data)?;
        let (_, _, orientation) = properties(&src);
        // SAFETY: every Create result is checked for null and owned by a
        // guard; the bitmap context draws into `rgba`, which has exactly
        // `w * 4 * h` bytes and outlives the context.
        unsafe {
            let image = CGImageSourceCreateImageAtIndex(src.0, 0, std::ptr::null());
            if image.is_null() {
                return Err(corrupt(FORMAT, "ImageIO could not decode the image"));
            }
            struct Img(CFTypeRef);
            impl Drop for Img {
                fn drop(&mut self) {
                    // SAFETY: owned reference from CGImageSourceCreateImageAtIndex.
                    unsafe { CGImageRelease(self.0) };
                }
            }
            let image = Img(image);
            let (w, h) = (CGImageGetWidth(image.0), CGImageGetHeight(image.0));
            let (w32, h32) = (
                u32::try_from(w).map_err(|_| corrupt(FORMAT, "bad width"))?,
                u32::try_from(h).map_err(|_| corrupt(FORMAT, "bad height"))?,
            );
            let len = check_size(w32, h32, PixelFormat::Rgba8)?;
            cancel.check()?;
            let mut rgba = util::alloc(len, w32, h32)?;
            let space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
            if space.is_null() {
                return Err(corrupt(FORMAT, "no sRGB color space"));
            }
            let ctx = CGBitmapContextCreate(
                rgba.as_mut_ptr().cast(),
                w,
                h,
                8,
                w * 4,
                space,
                RGBA_PREMULTIPLIED,
            );
            CGColorSpaceRelease(space);
            if ctx.is_null() {
                return Err(corrupt(FORMAT, "cannot create a bitmap context"));
            }
            let rect = CGRect {
                origin: CGPoint { x: 0.0, y: 0.0 },
                size: CGSize {
                    width: w as f64,
                    height: h as f64,
                },
            };
            CGContextDrawImage(ctx, rect, image.0);
            CGContextRelease(ctx);
            // The context stores premultiplied alpha.
            for p in rgba.chunks_exact_mut(4) {
                let a = u32::from(p[3]);
                if a != 0 && a != 255 {
                    for c in &mut p[..3] {
                        *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
            // ImageIO already converted to sRGB while drawing.
            Ok((finish(w32, h32, rgba, None)?, orientation))
        }
    }
}

#[cfg(test)]
mod fit_tests {
    use super::*;

    #[test]
    fn thumbnail_fits_only_as_a_true_stand_in() {
        let full = (9504, 6336);
        assert!(thumbnail_fits((1616, 1080), full, (1280, 720)));
        assert!(thumbnail_fits((1616, 1080), full, (2000, 1080)));
        assert!(
            !thumbnail_fits((1616, 1080), full, (2000, 1200)),
            "too small"
        );
        assert!(!thumbnail_fits((160, 90), full, (100, 100)), "16:9 for 3:2");
        assert!(!thumbnail_fits(full, full, (10, 10)), "not smaller");
        assert!(!thumbnail_fits((0, 0), full, (1, 1)));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Decodes the `.hif` files (Sony's name for HEIF) in the folder named
    /// by `PANDITE_HIF_SAMPLES` through Windows Imaging Component and
    /// prints the codec's native bit depth. Opt-in: needs the HEIF and HEVC
    /// extensions of the operating system and sample files.
    #[test]
    #[ignore = "needs the HEIF and HEVC codecs of the operating system and HIF samples"]
    fn hif_decodes_through_os_codec() {
        let Ok(dir) = std::env::var("PANDITE_HIF_SAMPLES") else {
            println!("skip: PANDITE_HIF_SAMPLES is not set");
            return;
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let folder = root.join(dir);
        let mut files: Vec<_> = std::fs::read_dir(&folder)
            .expect("sample folder")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("hif"))
            })
            .collect();
        files.sort();
        assert!(!files.is_empty(), "no .HIF files in {}", folder.display());
        for path in files {
            let data = std::fs::read(&path).unwrap();
            assert!(is_heif(&data[..data.len().min(64)]), "HEIF brand");
            let (guid, bpp, channels) = os::source_format(&data).expect("WIC opens the file");
            // GUID_WICPixelFormat32bppBGR: 8 bits per channel and one
            // padding byte.
            let name = if guid.eq_ignore_ascii_case("6FDDC324-4E03-4BFE-B185-3D77768DC90E") {
                "32bppBGR, 8 bits per channel"
            } else {
                "other"
            };
            // The encoded HEVC stream: hvcC (ISO/IEC 14496-15) byte 16 holds
            // chroma_format_idc, bytes 17 and 18 the bit depths minus 8.
            let coded = data
                .windows(4)
                .take(1 << 20)
                .position(|w| w == b"hvcC")
                .and_then(|i| data.get(i + 4..i + 4 + 19))
                .map(|c| {
                    let chroma = ["4:0:0", "4:2:0", "4:2:2", "4:4:4"][usize::from(c[16] & 3)];
                    format!("{chroma}, {} / {} bits", (c[17] & 7) + 8, (c[18] & 7) + 8)
                })
                .unwrap_or_else(|| "no hvcC box".into());
            let start = std::time::Instant::now();
            let source = Source::from_bytes(data, Some(path.clone()));
            let content = HeicDecoder
                .decode(&source, &DecodeOptions::default())
                .expect("WIC decodes the file");
            let img = content.first_image();
            println!(
                "{}: coded HEVC {coded}; WIC native format {guid} ({name}), {bpp} bits per pixel, {channels} channels; decoded {}x{} {:?} in {:.2}s",
                path.file_name().unwrap().to_string_lossy(),
                img.width(),
                img.height(),
                img.format(),
                start.elapsed().as_secs_f64()
            );
        }
    }
}
