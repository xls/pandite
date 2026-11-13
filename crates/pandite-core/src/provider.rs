//! Decoder and encoder provider traits and the registry that holds them.
//!
//! A provider is a small stateless object compiled into the binary. Format
//! crates construct their providers and add them to a [`Registry`] at
//! startup. Nothing loads code at runtime.

use crate::error::{Error, Result};
use crate::image::{Content, ImageBuf, ImageMeta};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Number of leading bytes handed to [`DecoderProvider::sniff`].
pub const SNIFF_LEN: usize = 64;

/// A shared flag that asks long work to stop. Cloning shares the flag.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token that is not cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the work to stop.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// True once [`CancelToken::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// `Err(Error::Cancelled)` once cancelled. Call between rows or frames.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Shared file bytes, held the way they arrived so no copy is made.
#[derive(Clone, Debug)]
enum Bytes {
    Shared(Arc<[u8]>),
    Owned(Arc<Vec<u8>>),
}

/// The bytes of one file plus where they came from.
#[derive(Clone, Debug)]
pub struct Source {
    path: Option<PathBuf>,
    bytes: Bytes,
}

impl Source {
    /// Read a whole file.
    pub fn from_path(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let bytes = std::fs::read(&path)?;
        Ok(Self::from_vec(bytes, Some(path)))
    }

    /// Wrap bytes that are already in memory. `path` is only a hint for
    /// extension matching and for platform codecs that want a file name.
    /// A `Vec` passed here is copied into the shared buffer; use
    /// [`Source::from_vec`] to keep it as it is.
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>, path: Option<PathBuf>) -> Self {
        Self {
            path,
            bytes: Bytes::Shared(bytes.into()),
        }
    }

    /// Wrap a buffer without copying it (a file read into a `Vec`).
    pub fn from_vec(bytes: Vec<u8>, path: Option<PathBuf>) -> Self {
        Self {
            path,
            bytes: Bytes::Owned(Arc::new(bytes)),
        }
    }

    /// The file contents.
    pub fn bytes(&self) -> &[u8] {
        match &self.bytes {
            Bytes::Shared(b) => b,
            Bytes::Owned(b) => b,
        }
    }

    /// The originating path when known.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The lowercase extension without the dot, when the path has one.
    pub fn extension(&self) -> Option<String> {
        extension_of(self.path.as_deref()?)
    }

    /// The first [`SNIFF_LEN`] bytes (fewer for short files).
    pub fn header(&self) -> &[u8] {
        let bytes = self.bytes();
        &bytes[..bytes.len().min(SNIFF_LEN)]
    }
}

/// The lowercase extension of `path` without the dot.
pub fn extension_of(path: &Path) -> Option<String> {
    Some(path.extension()?.to_str()?.to_ascii_lowercase())
}

/// Hints and limits for one decode call.
#[derive(Clone, Debug, Default)]
pub struct DecodeOptions {
    /// The caller only needs an image about this large (thumbnails).
    /// Decoders that can decode at reduced size cheaply (JPEG scaling, SVG,
    /// RAW embedded previews) use it. The result is never smaller than the
    /// target on both axes unless the source itself is smaller. Others
    /// ignore it and return full size.
    pub target_size: Option<(u32, u32)>,
    /// Return only the first frame of an animation as a still.
    pub first_frame_only: bool,
    /// The result is only displayed: decoders that develop deep data
    /// (camera RAW) deliver [`crate::PixelFormat::Rgba8`] instead of
    /// `Rgba16`, half the memory and upload. Others ignore it. Leave it
    /// off where precision matters (conversion, saving).
    pub prefer_8bit: bool,
    /// Lets the caller abandon a decode that is no longer needed.
    pub cancel: CancelToken,
}

/// Reads one or more file formats into [`Content`].
pub trait DecoderProvider: Send + Sync {
    /// Stable lowercase id, for example `"jpeg"`. Unique in a registry.
    fn id(&self) -> &'static str;

    /// Human-readable name for dialogs, for example `"JPEG image"`.
    fn name(&self) -> &'static str;

    /// Lowercase file extensions without the dot.
    fn extensions(&self) -> &'static [&'static str];

    /// True when `header` (the first [`SNIFF_LEN`] bytes) carries this
    /// format's signature. Formats with no reliable signature return false
    /// and are matched by extension only.
    fn sniff(&self, header: &[u8]) -> bool;

    /// Read size and basic facts without decoding pixels.
    fn probe(&self, source: &Source) -> Result<ImageMeta>;

    /// True when a decode of `source` with [`DecodeOptions::target_size`]
    /// returns a smaller image at a fraction of the cost of a full decode
    /// (DCT scaling, embedded previews, vector rendering). Callers use it
    /// to decide whether a fast first stage is worth running beside the
    /// full decode. Default: false.
    fn reduced_decode(&self, source: &Source) -> bool {
        let _ = source;
        false
    }

    /// Decode the file. Pixels come back upright: any stored orientation is
    /// already applied. Must be safe to call from several threads at once.
    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content>;
}

/// Settings for one encode call.
#[derive(Clone, Debug)]
pub struct EncodeOptions {
    /// 1 (smallest) to 100 (best). Ignored by lossless-only formats.
    pub quality: u8,
    /// Ask for lossless output where the format offers a choice (WebP).
    pub lossless: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            quality: 90,
            lossless: false,
        }
    }
}

/// Writes still images in one file format.
pub trait EncoderProvider: Send + Sync {
    /// Stable lowercase id, for example `"png"`. Unique in a registry.
    fn id(&self) -> &'static str;

    /// Human-readable name for dialogs, for example `"PNG image"`.
    fn name(&self) -> &'static str;

    /// Lowercase file extensions without the dot. The first is the default.
    fn extensions(&self) -> &'static [&'static str];

    /// True when the format can store an alpha channel.
    fn supports_alpha(&self) -> bool;

    /// True when [`EncodeOptions::quality`] changes the output.
    fn has_quality(&self) -> bool;

    /// Encode `image` to `out`. Providers convert pixel formats they cannot
    /// store (for example flattening alpha onto white for JPEG).
    fn encode(&self, image: &ImageBuf, options: &EncodeOptions, out: &mut dyn Write) -> Result<()>;
}

/// The set of providers compiled into the application.
#[derive(Clone, Default)]
pub struct Registry {
    decoders: Vec<Arc<dyn DecoderProvider>>,
    encoders: Vec<Arc<dyn EncoderProvider>>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a decoder. A provider with the same id replaces the earlier one.
    pub fn add_decoder(&mut self, provider: Arc<dyn DecoderProvider>) {
        self.decoders.retain(|p| p.id() != provider.id());
        self.decoders.push(provider);
    }

    /// Add an encoder. A provider with the same id replaces the earlier one.
    pub fn add_encoder(&mut self, provider: Arc<dyn EncoderProvider>) {
        self.encoders.retain(|p| p.id() != provider.id());
        self.encoders.push(provider);
    }

    /// All decoders in registration order.
    pub fn decoders(&self) -> &[Arc<dyn DecoderProvider>] {
        &self.decoders
    }

    /// All encoders in registration order.
    pub fn encoders(&self) -> &[Arc<dyn EncoderProvider>] {
        &self.encoders
    }

    /// The decoder with this id.
    pub fn decoder_by_id(&self, id: &str) -> Option<&Arc<dyn DecoderProvider>> {
        self.decoders.iter().find(|p| p.id() == id)
    }

    /// The encoder with this id.
    pub fn encoder_by_id(&self, id: &str) -> Option<&Arc<dyn EncoderProvider>> {
        self.encoders.iter().find(|p| p.id() == id)
    }

    /// The encoder that writes files with this extension (no dot, any case).
    pub fn encoder_for_extension(&self, ext: &str) -> Option<&Arc<dyn EncoderProvider>> {
        let ext = ext.to_ascii_lowercase();
        self.encoders
            .iter()
            .find(|p| p.extensions().contains(&ext.as_str()))
    }

    /// True when some decoder lists this path's extension. Used to filter
    /// folder listings without opening files.
    pub fn can_open_path(&self, path: &Path) -> bool {
        extension_of(path).is_some_and(|ext| {
            self.decoders
                .iter()
                .any(|p| p.extensions().contains(&ext.as_str()))
        })
    }

    /// Every decodable extension, sorted and without duplicates.
    pub fn decodable_extensions(&self) -> Vec<&'static str> {
        let mut all: Vec<&'static str> = self
            .decoders
            .iter()
            .flat_map(|p| p.extensions().iter().copied())
            .collect();
        all.sort_unstable();
        all.dedup();
        all
    }

    /// Pick the decoder for a source. File signatures win over extensions,
    /// so a PNG named `.jpg` still opens. Extension is the fallback for
    /// formats without a signature.
    pub fn decoder_for(&self, source: &Source) -> Option<&Arc<dyn DecoderProvider>> {
        let header = source.header();
        if let Some(found) = self.decoders.iter().find(|p| p.sniff(header)) {
            return Some(found);
        }
        let ext = source.extension()?;
        self.decoders
            .iter()
            .find(|p| p.extensions().contains(&ext.as_str()))
    }

    /// Probe a source with the matching decoder.
    pub fn probe(&self, source: &Source) -> Result<ImageMeta> {
        self.require_decoder(source)?.probe(source)
    }

    /// Decode a source with the matching decoder.
    pub fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        self.require_decoder(source)?.decode(source, options)
    }

    fn require_decoder(&self, source: &Source) -> Result<&Arc<dyn DecoderProvider>> {
        self.decoder_for(source).ok_or_else(|| {
            Error::Unsupported(match source.path() {
                Some(path) => path.display().to_string(),
                None => "unrecognized data".into(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{ContentKind, Orientation};

    struct Fake {
        id: &'static str,
        exts: &'static [&'static str],
        magic: &'static [u8],
    }

    impl DecoderProvider for Fake {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            "fake"
        }
        fn extensions(&self) -> &'static [&'static str] {
            self.exts
        }
        fn sniff(&self, header: &[u8]) -> bool {
            !self.magic.is_empty() && header.starts_with(self.magic)
        }
        fn probe(&self, _source: &Source) -> Result<ImageMeta> {
            Ok(ImageMeta {
                width: 1,
                height: 1,
                format: self.id,
                kind: ContentKind::Still,
                frame_count: 1,
                has_alpha: false,
                orientation: Orientation::Normal,
            })
        }
        fn decode(&self, _source: &Source, options: &DecodeOptions) -> Result<Content> {
            options.cancel.check()?;
            Ok(Content::Still(ImageBuf::solid_rgba8(1, 1, [0; 4])?))
        }
    }

    fn registry() -> Registry {
        let mut reg = Registry::new();
        reg.add_decoder(Arc::new(Fake {
            id: "aaa",
            exts: &["aaa", "a"],
            magic: b"AAA",
        }));
        reg.add_decoder(Arc::new(Fake {
            id: "raw",
            exts: &["bbb"],
            magic: b"",
        }));
        reg
    }

    #[test]
    fn registry_prefers_signature_over_extension() {
        let reg = registry();
        let src = Source::from_bytes(b"AAA....".to_vec(), Some("wrong.BBB".into()));
        assert_eq!(reg.decoder_for(&src).unwrap().id(), "aaa");
        let src = Source::from_bytes(b"????".to_vec(), Some("x.BBB".into()));
        assert_eq!(reg.decoder_for(&src).unwrap().id(), "raw");
        let src = Source::from_bytes(b"????".to_vec(), Some("x.zzz".into()));
        assert!(reg.decoder_for(&src).is_none());
        assert!(matches!(reg.probe(&src), Err(Error::Unsupported(_))));
    }

    #[test]
    fn registry_lists_and_replaces() {
        let mut reg = registry();
        assert_eq!(reg.decodable_extensions(), vec!["a", "aaa", "bbb"]);
        assert!(reg.can_open_path(Path::new("C:/x/photo.AAA")));
        assert!(!reg.can_open_path(Path::new("C:/x/photo")));
        reg.add_decoder(Arc::new(Fake {
            id: "aaa",
            exts: &["new"],
            magic: b"",
        }));
        assert_eq!(reg.decoders().len(), 2);
        assert_eq!(reg.decoder_by_id("aaa").unwrap().extensions(), &["new"]);
    }

    #[test]
    fn cancel_token_stops_decode() {
        let reg = registry();
        let src = Source::from_bytes(b"AAA".to_vec(), None);
        let options = DecodeOptions::default();
        assert!(reg.decode(&src, &options).is_ok());
        options.cancel.cancel();
        assert!(matches!(reg.decode(&src, &options), Err(Error::Cancelled)));
    }

    #[test]
    fn source_header_and_extension() {
        let src = Source::from_bytes(vec![7u8; 200], Some("A/B/Pic.JPeG".into()));
        assert_eq!(src.header().len(), SNIFF_LEN);
        assert_eq!(src.extension().as_deref(), Some("jpeg"));
        let short = Source::from_bytes(vec![1u8, 2], None);
        assert_eq!(short.header(), &[1, 2]);
        assert_eq!(short.extension(), None);
        // from_vec keeps the buffer: no copy of a large file.
        let data = vec![9u8; 300];
        let ptr = data.as_ptr();
        let owned = Source::from_vec(data, Some("x.PNG".into()));
        assert_eq!(owned.bytes().as_ptr(), ptr);
        assert_eq!(owned.header().len(), SNIFF_LEN);
        assert_eq!(owned.clone().bytes().len(), 300);
        assert_eq!(owned.extension().as_deref(), Some("png"));
    }
}
