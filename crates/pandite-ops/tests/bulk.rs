//! Gate G3: bulk resize and reformat through the registry.
//!
//! The providers here are fakes: a trivial raw format ("RAWI": magic, width,
//! height, RGBA8 pixels), a second one with another magic ("RAWO") to test
//! reformatting, an animated decoder and a decoder that panics.

use pandite_core::{
    Animation, CancelToken, Content, ContentKind, DecodeOptions, DecoderProvider, EncodeOptions,
    EncoderProvider, Error, Frame, ImageBuf, ImageMeta, LoopCount, Orientation, PixelFormat,
    Registry, Result, Source,
};
use pandite_ops::{
    run_bulk, BulkJob, Destination, FileOutcome, OverwritePolicy, Progress, ResizeSpec,
};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RAWI: &[u8; 4] = b"RAWI";
const RAWO: &[u8; 4] = b"RAWO";
const ANIM: &[u8; 4] = b"ANIM";

fn raw_bytes(magic: &[u8; 4], img: &ImageBuf) -> Vec<u8> {
    let mut out = magic.to_vec();
    out.extend_from_slice(&img.width().to_le_bytes());
    out.extend_from_slice(&img.height().to_le_bytes());
    out.extend_from_slice(img.data());
    out
}

fn parse_raw(magic: &[u8; 4], bytes: &[u8]) -> Result<ImageBuf> {
    if bytes.len() < 12 || &bytes[..4] != magic {
        return Err(Error::Corrupt("not a raw test image".into()));
    }
    let w = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let h = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let data = bytes[12..].to_vec();
    if data.len() as u64 != u64::from(w) * u64::from(h) * 4 {
        return Err(Error::Corrupt("pixel data has the wrong length".into()));
    }
    ImageBuf::new(w, h, PixelFormat::Rgba8, data)
}

/// Records which threads decode and how many decodes overlap.
#[derive(Default)]
struct DecodeLog {
    threads: Mutex<Vec<ThreadId>>,
    active: AtomicUsize,
    peak: AtomicUsize,
    /// Hold each decode until a second one runs at the same time (or a
    /// timeout passes), proving that files run in parallel.
    wait_for_peer: AtomicBool,
    first_frame_requests: AtomicUsize,
}

impl DecodeLog {
    fn enter(&self, options: &DecodeOptions) {
        self.threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
        if options.first_frame_only {
            self.first_frame_requests.fetch_add(1, Ordering::SeqCst);
        }
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        if self.wait_for_peer.load(Ordering::SeqCst) {
            let start = Instant::now();
            while self.peak.load(Ordering::SeqCst) < 2 && start.elapsed() < Duration::from_secs(20)
            {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    fn leave(&self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

struct RawDecoder {
    id: &'static str,
    magic: &'static [u8; 4],
    exts: &'static [&'static str],
    log: Arc<DecodeLog>,
}

impl DecoderProvider for RawDecoder {
    fn id(&self) -> &'static str {
        self.id
    }
    fn name(&self) -> &'static str {
        "raw test image"
    }
    fn extensions(&self) -> &'static [&'static str] {
        self.exts
    }
    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(self.magic)
    }
    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let img = parse_raw(self.magic, source.bytes())?;
        Ok(ImageMeta {
            width: img.width(),
            height: img.height(),
            format: self.id,
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: true,
            orientation: Orientation::Normal,
        })
    }
    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        self.log.enter(options);
        let result = parse_raw(self.magic, source.bytes()).map(Content::Still);
        self.log.leave();
        result
    }
}

/// Two frames, red then blue, of the size in the header. Always returns the
/// full animation, even when asked for the first frame only.
struct AnimDecoder {
    log: Arc<DecodeLog>,
}

impl DecoderProvider for AnimDecoder {
    fn id(&self) -> &'static str {
        "anim"
    }
    fn name(&self) -> &'static str {
        "animated test image"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["anim"]
    }
    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(ANIM)
    }
    fn probe(&self, _source: &Source) -> Result<ImageMeta> {
        Err(Error::Unsupported("probe".into()))
    }
    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        self.log.enter(options);
        self.log.leave();
        let b = source.bytes();
        let w = u32::from_le_bytes(b[4..8].try_into().unwrap());
        let h = u32::from_le_bytes(b[8..12].try_into().unwrap());
        let frame = |rgba| Frame {
            image: ImageBuf::solid_rgba8(w, h, rgba).unwrap(),
            delay: Duration::from_millis(50),
        };
        let anim = Animation::new(
            vec![frame([255, 0, 0, 255]), frame([0, 0, 255, 255])],
            LoopCount::Infinite,
        )?;
        Ok(Content::Animation(anim))
    }
}

/// Decodes files that start with "BOOM" by panicking.
struct PanicDecoder;

impl DecoderProvider for PanicDecoder {
    fn id(&self) -> &'static str {
        "boom"
    }
    fn name(&self) -> &'static str {
        "panicking test decoder"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["boom"]
    }
    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(b"BOOM")
    }
    fn probe(&self, _source: &Source) -> Result<ImageMeta> {
        Err(Error::Unsupported("probe".into()))
    }
    fn decode(&self, _source: &Source, _options: &DecodeOptions) -> Result<Content> {
        panic!("test decoder panics on purpose");
    }
}

struct RawEncoder {
    id: &'static str,
    magic: &'static [u8; 4],
    exts: &'static [&'static str],
}

impl EncoderProvider for RawEncoder {
    fn id(&self) -> &'static str {
        self.id
    }
    fn name(&self) -> &'static str {
        "raw test image"
    }
    fn extensions(&self) -> &'static [&'static str] {
        self.exts
    }
    fn supports_alpha(&self) -> bool {
        true
    }
    fn has_quality(&self) -> bool {
        false
    }
    fn encode(
        &self,
        image: &ImageBuf,
        _options: &EncodeOptions,
        out: &mut dyn Write,
    ) -> Result<()> {
        if image.format() != PixelFormat::Rgba8 {
            return Err(Error::Unsupported(
                "raw test encoder writes Rgba8 only".into(),
            ));
        }
        out.write_all(&raw_bytes(self.magic, image))?;
        Ok(())
    }
}

fn registry_with(log: &Arc<DecodeLog>) -> Registry {
    let mut reg = Registry::new();
    for (id, magic, exts) in [
        ("rawi", RAWI, &["rawi"] as &'static [&'static str]),
        ("rawo", RAWO, &["rawo", "ro"]),
    ] {
        reg.add_decoder(Arc::new(RawDecoder {
            id,
            magic,
            exts,
            log: Arc::clone(log),
        }));
        reg.add_encoder(Arc::new(RawEncoder { id, magic, exts }));
    }
    reg.add_decoder(Arc::new(AnimDecoder {
        log: Arc::clone(log),
    }));
    reg.add_decoder(Arc::new(PanicDecoder));
    reg
}

fn registry() -> Registry {
    registry_with(&Arc::new(DecodeLog::default()))
}

/// A unique folder under the system temp folder, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("pandite-ops-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    fn join(&self, part: &str) -> PathBuf {
        self.0.join(part)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_raw(path: &Path, img: &ImageBuf) -> PathBuf {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, raw_bytes(RAWI, img)).unwrap();
    path.to_path_buf()
}

fn read_any(path: &Path) -> ImageBuf {
    let bytes = fs::read(path).unwrap();
    parse_raw(RAWI, &bytes)
        .or_else(|_| parse_raw(RAWO, &bytes))
        .unwrap()
}

/// Sorted file names in a folder.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn solid(w: u32, h: u32, rgba: [u8; 4]) -> ImageBuf {
    ImageBuf::solid_rgba8(w, h, rgba).unwrap()
}

fn no_progress(_: &Progress<'_>) {}

fn written(outcome: &FileOutcome) -> (PathBuf, u32, u32) {
    match outcome {
        FileOutcome::Written {
            path,
            width,
            height,
        } => (path.clone(), *width, *height),
        other => panic!("expected a written file, got {other:?}"),
    }
}

#[test]
fn bulk_resize_percent_writes_outputs() {
    let tmp = TempDir::new("percent");
    let checker = ImageBuf::new(
        16,
        16,
        PixelFormat::Rgba8,
        (0..256u32)
            .flat_map(|i| {
                let v = if (i % 16 + i / 16) % 2 == 0 { 255 } else { 0 };
                [v, v, v, 255]
            })
            .collect(),
    )
    .unwrap();
    let inputs = vec![
        write_raw(&tmp.join("in/a.rawi"), &solid(40, 20, [200, 10, 10, 255])),
        write_raw(&tmp.join("in/b.rawi"), &solid(30, 30, [10, 200, 10, 128])),
        write_raw(&tmp.join("in/c.rawi"), &solid(11, 7, [10, 10, 200, 255])),
        write_raw(&tmp.join("in/d.rawi"), &checker),
    ];
    let originals: Vec<Vec<u8>> = inputs.iter().map(|p| fs::read(p).unwrap()).collect();
    let out = tmp.join("out/nested");
    let job = BulkJob {
        resize: Some(ResizeSpec::Percent(50.0)),
        destination: Destination::Directory(out.clone()),
        ..BulkJob::new(inputs.clone())
    };
    let report = run_bulk(&job, &registry(), &CancelToken::new(), &no_progress).unwrap();
    assert_eq!(report.files.len(), 4);
    assert_eq!((report.written(), report.failed()), (4, 0));
    assert!(!report.cancelled);
    let expect = [
        ("a.rawi", 20, 10, [200, 10, 10, 255]),
        ("b.rawi", 15, 15, [10, 200, 10, 128]),
        // 11 x 7 at 50 percent rounds to 6 x 4.
        ("c.rawi", 6, 4, [10, 10, 200, 255]),
    ];
    for (file, (name, w, h, rgba)) in report.files.iter().zip(expect) {
        let (path, width, height) = written(&file.outcome);
        assert_eq!(path, out.join(name));
        assert_eq!((width, height), (w, h));
        let img = read_any(&path);
        assert_eq!((img.width(), img.height()), (w, h));
        assert!(img.data().chunks_exact(4).all(|p| p == rgba), "{name}");
    }
    // The resize inside the batch is the filtered one: a 1-pixel
    // checkerboard halves to flat linear-light gray.
    let (path, _, _) = written(&report.files[3].outcome);
    let gray = read_any(&path);
    assert_eq!((gray.width(), gray.height()), (8, 8));
    assert!(gray
        .data()
        .chunks_exact(4)
        .all(|p| p == [188, 188, 188, 255]));
    // Exactly the outputs: no temporary files left, inputs untouched.
    assert_eq!(names(&out), vec!["a.rawi", "b.rawi", "c.rawi", "d.rawi"]);
    for (input, original) in inputs.iter().zip(&originals) {
        assert_eq!(&fs::read(input).unwrap(), original);
    }
    assert_eq!(names(&tmp.join("in")).len(), 4);
}

#[test]
fn bulk_reformat_changes_extension() {
    let tmp = TempDir::new("reformat");
    let pattern = ImageBuf::new(
        3,
        2,
        PixelFormat::Rgba8,
        (0..24).map(|i| (i * 10) as u8).collect(),
    )
    .unwrap();
    let inputs = vec![
        write_raw(&tmp.join("src/photo.rawi"), &pattern),
        write_raw(&tmp.join("src/Other.RAWI"), &solid(5, 5, [1, 2, 3, 4])),
    ];
    // By encoder id, beside the sources, with a name pattern.
    let job = BulkJob {
        output_format: Some("rawo".into()),
        name_pattern: "{name}_conv".into(),
        ..BulkJob::new(inputs.clone())
    };
    let report = run_bulk(&job, &registry(), &CancelToken::new(), &no_progress).unwrap();
    assert_eq!(report.written(), 2);
    let (path, w, h) = written(&report.files[0].outcome);
    assert_eq!(path, tmp.join("src").join("photo_conv.rawo"));
    assert_eq!((w, h), (3, 2));
    let bytes = fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], RAWO);
    // No resize: the pixels are carried over exactly.
    assert_eq!(read_any(&path).data(), pattern.data());
    let (path, _, _) = written(&report.files[1].outcome);
    assert_eq!(path, tmp.join("src").join("Other_conv.rawo"));

    // By extension alias ("ro" belongs to the rawo encoder), into a folder,
    // with the index placeholder; the default extension is used.
    let job = BulkJob {
        output_format: Some(".RO".into()),
        name_pattern: "{n}-{name}".into(),
        destination: Destination::Directory(tmp.join("out")),
        ..BulkJob::new(inputs.clone())
    };
    let report = run_bulk(&job, &registry(), &CancelToken::new(), &no_progress).unwrap();
    assert_eq!(report.written(), 2);
    assert_eq!(
        names(&tmp.join("out")),
        vec!["1-photo.rawo", "2-Other.rawo"]
    );

    // Keeping the format keeps the input's extension spelling.
    let job = BulkJob {
        name_pattern: "{name}_copy".into(),
        ..BulkJob::new(vec![inputs[1].clone()])
    };
    let report = run_bulk(&job, &registry(), &CancelToken::new(), &no_progress).unwrap();
    let (path, _, _) = written(&report.files[0].outcome);
    assert_eq!(path, tmp.join("src").join("Other_copy.RAWI"));
    assert_eq!(&fs::read(&path).unwrap()[..4], RAWI);

    // Job-level errors stop before touching any file.
    let before = names(&tmp.join("src"));
    let bad_format = BulkJob {
        output_format: Some("xyz".into()),
        ..BulkJob::new(inputs.clone())
    };
    assert!(matches!(
        run_bulk(&bad_format, &registry(), &CancelToken::new(), &no_progress),
        Err(Error::Unsupported(_))
    ));
    let bad_pattern = BulkJob {
        name_pattern: "{name}/x".into(),
        ..BulkJob::new(inputs)
    };
    assert!(matches!(
        run_bulk(&bad_pattern, &registry(), &CancelToken::new(), &no_progress),
        Err(Error::Invalid(_))
    ));
    assert_eq!(names(&tmp.join("src")), before);
}

#[test]
fn bulk_best_fit_never_enlarges_unless_asked() {
    let tmp = TempDir::new("fit");
    let small = solid(40, 20, [50, 60, 70, 255]);
    let inputs = vec![
        write_raw(&tmp.join("in/big.rawi"), &solid(200, 100, [1, 1, 1, 255])),
        write_raw(&tmp.join("in/small.rawi"), &small),
        write_raw(&tmp.join("in/tall.rawi"), &solid(50, 300, [2, 2, 2, 255])),
    ];
    let run = |enlarge: bool, dir: &str| {
        let job = BulkJob {
            resize: Some(ResizeSpec::Fit {
                width: 100,
                height: 100,
                enlarge,
            }),
            destination: Destination::Directory(tmp.join(dir)),
            ..BulkJob::new(inputs.clone())
        };
        let report = run_bulk(&job, &registry(), &CancelToken::new(), &no_progress).unwrap();
        assert_eq!(report.written(), 3);
        report
            .files
            .iter()
            .map(|f| {
                let (path, w, h) = written(&f.outcome);
                let img = read_any(&path);
                assert_eq!((img.width(), img.height()), (w, h));
                img
            })
            .collect::<Vec<_>>()
    };
    let kept = run(false, "keep");
    let dims: Vec<_> = kept.iter().map(|i| (i.width(), i.height())).collect();
    // 50 x 300 fits as 16.67 x 100, rounded to 17 x 100.
    assert_eq!(dims, vec![(100, 50), (40, 20), (17, 100)]);
    assert_eq!(kept[1].data(), small.data());
    let grown = run(true, "grow");
    let dims: Vec<_> = grown.iter().map(|i| (i.width(), i.height())).collect();
    assert_eq!(dims, vec![(100, 50), (100, 50), (17, 100)]);
    assert!(grown[1]
        .data()
        .chunks_exact(4)
        .all(|p| p == [50, 60, 70, 255]));
}

#[test]
fn bulk_does_not_overwrite_by_default() {
    let tmp = TempDir::new("overwrite");
    let red = solid(4, 4, [255, 0, 0, 255]);
    let a = write_raw(&tmp.join("src/a.rawi"), &red);
    let b = write_raw(&tmp.join("src/b.rawi"), &solid(4, 4, [0, 0, 255, 255]));
    let out = tmp.join("out");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("a.rawi"), b"keep me").unwrap();
    let job = BulkJob {
        destination: Destination::Directory(out.clone()),
        ..BulkJob::new(vec![a.clone(), b.clone()])
    };
    assert_eq!(job.overwrite, OverwritePolicy::Rename);

    // Default: the existing file stays, the output takes a numeric suffix.
    let report = run_bulk(&job, &registry(), &CancelToken::new(), &no_progress).unwrap();
    assert_eq!(written(&report.files[0].outcome).0, out.join("a_1.rawi"));
    assert_eq!(written(&report.files[1].outcome).0, out.join("b.rawi"));
    assert_eq!(fs::read(out.join("a.rawi")).unwrap(), b"keep me");
    // Running again overwrites nothing either.
    let report = run_bulk(&job, &registry(), &CancelToken::new(), &no_progress).unwrap();
    assert_eq!(written(&report.files[0].outcome).0, out.join("a_2.rawi"));
    assert_eq!(written(&report.files[1].outcome).0, out.join("b_1.rawi"));
    assert_eq!(
        names(&out),
        vec!["a.rawi", "a_1.rawi", "a_2.rawi", "b.rawi", "b_1.rawi"]
    );
    assert_eq!(fs::read(out.join("a.rawi")).unwrap(), b"keep me");

    // Beside the source in the same format: the source itself is protected.
    let source_bytes = fs::read(&a).unwrap();
    let beside = BulkJob::new(vec![a.clone()]);
    let report = run_bulk(&beside, &registry(), &CancelToken::new(), &no_progress).unwrap();
    assert_eq!(
        written(&report.files[0].outcome).0,
        tmp.join("src").join("a_1.rawi")
    );
    assert_eq!(fs::read(&a).unwrap(), source_bytes);

    // Skip: report and leave the file alone.
    let skip = BulkJob {
        overwrite: OverwritePolicy::Skip,
        ..job.clone()
    };
    let report = run_bulk(&skip, &registry(), &CancelToken::new(), &no_progress).unwrap();
    assert_eq!(report.skipped(), 2);
    assert!(matches!(
        &report.files[0].outcome,
        FileOutcome::Skipped { existing } if *existing == out.join("a.rawi")
    ));
    assert_eq!(fs::read(out.join("a.rawi")).unwrap(), b"keep me");
    assert_eq!(names(&out).len(), 5);

    // Overwrite only when asked.
    let replace = BulkJob {
        overwrite: OverwritePolicy::Overwrite,
        ..job.clone()
    };
    let report = run_bulk(&replace, &registry(), &CancelToken::new(), &no_progress).unwrap();
    assert_eq!(written(&report.files[0].outcome).0, out.join("a.rawi"));
    assert_eq!(read_any(&out.join("a.rawi")).data(), red.data());
    assert_eq!(names(&out).len(), 5);

    // Two inputs with the same name never share one output, whatever the
    // policy, and the first input gets the plain name.
    let twin = write_raw(&tmp.join("src2/a.rawi"), &solid(2, 2, [9, 9, 9, 255]));
    for policy in [OverwritePolicy::Rename, OverwritePolicy::Overwrite] {
        let dir = tmp.join(&format!("twins-{policy:?}"));
        let job = BulkJob {
            destination: Destination::Directory(dir.clone()),
            overwrite: policy,
            threads: 2,
            ..BulkJob::new(vec![a.clone(), twin.clone()])
        };
        let report = run_bulk(&job, &registry(), &CancelToken::new(), &no_progress).unwrap();
        assert_eq!(written(&report.files[0].outcome).0, dir.join("a.rawi"));
        assert_eq!(written(&report.files[1].outcome).0, dir.join("a_1.rawi"));
        assert_eq!(read_any(&dir.join("a_1.rawi")).width(), 2);
    }
    // No temporary files anywhere.
    for dir in ["src", "src2", "out"] {
        assert!(names(&tmp.join(dir)).iter().all(|n| !n.ends_with(".tmp")));
    }
}

#[test]
fn bulk_reports_per_file_errors_and_continues() {
    let tmp = TempDir::new("errors");
    let good1 = write_raw(&tmp.join("in/good1.rawi"), &solid(6, 6, [1, 2, 3, 255]));
    let missing = tmp.join("in/missing.rawi");
    let truncated = tmp.join("in/truncated.rawi");
    let mut bytes = raw_bytes(RAWI, &solid(6, 6, [0; 4]));
    bytes.truncate(40);
    fs::write(&truncated, bytes).unwrap();
    let unknown = tmp.join("in/notes.txt");
    fs::write(&unknown, b"just text").unwrap();
    let garbage = tmp.join("in/garbage.rawi");
    fs::write(&garbage, b"hello").unwrap();
    let boom = tmp.join("in/crash.boom");
    fs::write(&boom, b"BOOM").unwrap();
    let good2 = write_raw(&tmp.join("in/good2.rawi"), &solid(6, 6, [4, 5, 6, 255]));
    let inputs = vec![
        good1,
        missing.clone(),
        truncated,
        unknown,
        garbage,
        boom,
        good2,
        missing,
    ];
    let calls = AtomicUsize::new(0);
    let out = tmp.join("out");
    let job = BulkJob {
        output_format: Some("rawo".into()),
        resize: Some(ResizeSpec::Scale(0.5)),
        destination: Destination::Directory(out.clone()),
        threads: 3,
        ..BulkJob::new(inputs)
    };
    let report = run_bulk(&job, &registry(), &CancelToken::new(), &|_: &Progress<
        '_,
    >| {
        calls.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 8);
    let kinds: Vec<&str> = report
        .files
        .iter()
        .map(|f| match &f.outcome {
            FileOutcome::Written { .. } => "written",
            FileOutcome::Failed(Error::Io(_)) => "io",
            FileOutcome::Failed(Error::Corrupt(_)) => "corrupt",
            FileOutcome::Failed(Error::Unsupported(_)) => "unsupported",
            FileOutcome::Failed(Error::Backend(_)) => "backend",
            FileOutcome::Failed(Error::Invalid(_)) => "invalid",
            other => panic!("unexpected outcome {other:?}"),
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "written",
            "io",
            "corrupt",
            "unsupported",
            "corrupt",
            "backend",
            "written",
            // The same path listed twice is refused instead of racing.
            "invalid",
        ]
    );
    assert_eq!((report.written(), report.failed()), (2, 6));
    assert_eq!(report.files[1].input, tmp.join("in/missing.rawi"));
    // Failed files leave nothing behind.
    assert_eq!(names(&out), vec!["good1.rawo", "good2.rawo"]);
    assert_eq!(read_any(&out.join("good2.rawo")).width(), 3);
}

#[test]
fn bulk_progress_and_cancel() {
    let tmp = TempDir::new("progress");
    let inputs: Vec<PathBuf> = (0..7)
        .map(|i| {
            write_raw(
                &tmp.join(&format!("in/f{i}.rawi")),
                &solid(8, 8, [i, 0, 0, 255]),
            )
        })
        .collect();
    let caller = std::thread::current().id();

    // Progress: one call per file, never concurrent, counting up to the
    // total, on worker threads; decodes overlap, so files run in parallel.
    let log = Arc::new(DecodeLog::default());
    log.wait_for_peer.store(true, Ordering::SeqCst);
    let events = Mutex::new(Vec::new());
    let in_callback = AtomicBool::new(false);
    let job = BulkJob {
        destination: Destination::Directory(tmp.join("out")),
        threads: 4,
        ..BulkJob::new(inputs.clone())
    };
    let report = run_bulk(
        &job,
        &registry_with(&log),
        &CancelToken::new(),
        &|p: &Progress<'_>| {
            assert!(
                !in_callback.swap(true, Ordering::SeqCst),
                "callbacks overlapped"
            );
            events.lock().unwrap().push((
                p.done,
                p.total,
                p.index,
                p.input.to_path_buf(),
                p.outcome.is_written(),
                std::thread::current().id(),
            ));
            in_callback.store(false, Ordering::SeqCst);
        },
    )
    .unwrap();
    assert_eq!(report.written(), 7);
    let events = events.into_inner().unwrap();
    let dones: Vec<usize> = events.iter().map(|e| e.0).collect();
    assert_eq!(dones, (1..=7).collect::<Vec<_>>());
    assert!(events.iter().all(|e| e.1 == 7 && e.4 && e.3 == inputs[e.2]));
    let mut indices: Vec<usize> = events.iter().map(|e| e.2).collect();
    indices.sort_unstable();
    assert_eq!(indices, (0..7).collect::<Vec<_>>());
    assert!(
        events.iter().all(|e| e.5 != caller),
        "progress ran on the caller"
    );
    let decode_threads = log.threads.lock().unwrap().clone();
    assert_eq!(decode_threads.len(), 7);
    assert!(
        decode_threads.iter().all(|t| *t != caller),
        "decode ran on the caller"
    );
    assert!(
        log.peak.load(Ordering::SeqCst) >= 2,
        "files never ran in parallel"
    );

    // Cancel after two files: the rest are reported cancelled, not written.
    let token = CancelToken::new();
    let out = tmp.join("cancelled");
    let job = BulkJob {
        destination: Destination::Directory(out.clone()),
        threads: 1,
        ..BulkJob::new(inputs.clone())
    };
    let calls = AtomicUsize::new(0);
    let report = run_bulk(&job, &registry(), &token, &|p: &Progress<'_>| {
        calls.fetch_add(1, Ordering::SeqCst);
        if p.done == 2 {
            token.cancel();
        }
    })
    .unwrap();
    assert!(report.cancelled);
    assert_eq!(calls.load(Ordering::SeqCst), 7);
    assert_eq!((report.written(), report.cancelled_files()), (2, 5));
    assert!(report.files[..2].iter().all(|f| f.outcome.is_written()));
    assert!(report.files[2..]
        .iter()
        .all(|f| matches!(f.outcome, FileOutcome::Cancelled)));
    assert_eq!(names(&out), vec!["f0.rawi", "f1.rawi"]);

    // Cancelled before the start: nothing is decoded or written.
    let log = Arc::new(DecodeLog::default());
    let token = CancelToken::new();
    token.cancel();
    let out = tmp.join("never");
    let job = BulkJob {
        destination: Destination::Directory(out.clone()),
        ..BulkJob::new(inputs)
    };
    let report = run_bulk(&job, &registry_with(&log), &token, &no_progress).unwrap();
    assert_eq!(report.cancelled_files(), 7);
    assert!(log.threads.lock().unwrap().is_empty());
    assert!(names(&out).is_empty());
}

#[test]
fn bulk_animated_input_uses_first_frame() {
    let tmp = TempDir::new("anim");
    let path = tmp.join("clip.anim");
    let mut header = ANIM.to_vec();
    header.extend_from_slice(&6u32.to_le_bytes());
    header.extend_from_slice(&4u32.to_le_bytes());
    fs::write(&path, header).unwrap();
    let log = Arc::new(DecodeLog::default());
    let job = BulkJob {
        output_format: Some("rawi".into()),
        resize: Some(ResizeSpec::Exact {
            width: 3,
            height: 2,
        }),
        ..BulkJob::new(vec![path])
    };
    let report = run_bulk(
        &job,
        &registry_with(&log),
        &CancelToken::new(),
        &no_progress,
    )
    .unwrap();
    let (out, w, h) = written(&report.files[0].outcome);
    assert_eq!(out, tmp.join("clip.rawi"));
    assert_eq!((w, h), (3, 2));
    // The first frame is red; the second, blue, is ignored.
    let img = read_any(&out);
    assert!(img.data().chunks_exact(4).all(|p| p == [255, 0, 0, 255]));
    assert_eq!(log.first_frame_requests.load(Ordering::SeqCst), 1);
}

#[test]
fn bulk_converts_deep_images_for_8bit_encoders() {
    // A decoder that yields 16-bit pixels, and an encoder that only takes
    // 8-bit: the engine retries with an 8-bit copy.
    struct Deep;
    impl DecoderProvider for Deep {
        fn id(&self) -> &'static str {
            "deep"
        }
        fn name(&self) -> &'static str {
            "16-bit test image"
        }
        fn extensions(&self) -> &'static [&'static str] {
            &["deep"]
        }
        fn sniff(&self, _header: &[u8]) -> bool {
            false
        }
        fn probe(&self, _source: &Source) -> Result<ImageMeta> {
            Err(Error::Unsupported("probe".into()))
        }
        fn decode(&self, _source: &Source, _options: &DecodeOptions) -> Result<Content> {
            let px = [65535u16, 32768, 0, 65535];
            let data = (0..4).flat_map(|_| px).flat_map(u16::to_ne_bytes).collect();
            Ok(Content::Still(ImageBuf::new(
                2,
                2,
                PixelFormat::Rgba16,
                data,
            )?))
        }
    }
    let tmp = TempDir::new("deep");
    let path = tmp.join("pic.deep");
    fs::write(&path, b"anything").unwrap();
    let mut reg = registry();
    reg.add_decoder(Arc::new(Deep));
    let job = BulkJob {
        output_format: Some("rawi".into()),
        ..BulkJob::new(vec![path])
    };
    let report = run_bulk(&job, &reg, &CancelToken::new(), &no_progress).unwrap();
    let (out, _, _) = written(&report.files[0].outcome);
    let img = read_any(&out);
    assert!(img.data().chunks_exact(4).all(|p| p == [255, 128, 0, 255]));
}
