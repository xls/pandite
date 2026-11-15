//! Folder access in the macOS App Sandbox, with fake bookmarks and a disk
//! that refuses folder listings the way the sandbox does: the prompt for a
//! refused folder, its folder picker, grants that last across sessions,
//! and the dismissal. Outside the sandbox none of it shows.

mod support;

use pandite_app::app::{Handled, SLOW_FOLDER_CHECK};
use pandite_app::disk::StdDisk;
use pandite_app::platform::DialogKind;
use pandite_app::sandbox::{
    Bookmarks, Resolved, Store, ALLOW_BUTTON, ALLOW_MESSAGE, ALLOW_TITLE, GRANTS_FILE,
};
use pandite_core::{DrawCmd, Point};
use pandite_ui::{Action, InputEvent, Key, Modifiers, PointerButton};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use support::*;

/// A gate that blocks bookmark resolution until it opens.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    cv: Condvar,
}

impl Gate {
    fn closed() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn open(&self) {
        *lock(&self.open) = true;
        self.cv.notify_all();
    }

    fn wait(&self) {
        let mut open = lock(&self.open);
        while !*open {
            open = self.cv.wait(open).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// Bookmarks that hold the folder path as bytes. Resolving one waits for
/// the gate, when there is one, and starts access by letting the test disk
/// list `grants_listing`.
#[derive(Clone, Default)]
struct FakeBookmarks {
    created: Arc<Mutex<Vec<PathBuf>>>,
    gate: Option<Arc<Gate>>,
    disk: Option<(Arc<TestDisk>, &'static str)>,
}

impl Bookmarks for FakeBookmarks {
    fn create(&self, dir: &Path) -> Result<Vec<u8>, String> {
        lock(&self.created).push(dir.to_path_buf());
        Ok(dir.to_string_lossy().into_owned().into_bytes())
    }

    fn resolve(&self, bookmark: &[u8]) -> Result<Resolved, String> {
        if let Some(gate) = &self.gate {
            gate.wait();
        }
        if let Some((disk, folder)) = &self.disk {
            disk.allow_listing(folder);
        }
        Ok(Resolved {
            dir: PathBuf::from(String::from_utf8(bookmark.to_vec()).unwrap()),
            stale: false,
            access: true,
        })
    }
}

impl FakeBookmarks {
    fn created(&self) -> Vec<PathBuf> {
        lock(&self.created).clone()
    }
}

/// `root/photos` with three images, and a settings folder.
struct Scene {
    root: TempDir,
    config: TempDir,
    photos: PathBuf,
    images: Vec<PathBuf>,
}

impl Scene {
    fn new(tag: &str) -> Self {
        let root = TempDir::new(tag);
        let photos = root.join("photos");
        std::fs::create_dir_all(&photos).unwrap();
        let images: Vec<PathBuf> = ["a.png", "b.png", "c.png"]
            .iter()
            .map(|n| {
                let p = photos.join(n);
                write_png(&p, &numbered(4, 3));
                p
            })
            .collect();
        Self {
            root,
            config: TempDir::new(&format!("{tag}-config")),
            photos,
            images,
        }
    }

    fn grants(&self) -> PathBuf {
        self.config.join(GRANTS_FILE)
    }

    fn store(&self, bookmarks: FakeBookmarks) -> Arc<Store> {
        Store::open(Box::new(bookmarks), Some(self.grants()), Arc::new(StdDisk))
    }

    fn harness(&self, disk: Arc<TestDisk>, sandbox: Option<Arc<Store>>) -> Harness {
        Harness::with_disk(
            Config {
                open: Some(self.images[0].clone()),
                settings_path: Some(self.config.join("settings.ini")),
                sandbox,
                ..Config::default()
            },
            disk,
        )
    }
}

/// Click the text run that reads `label` in a fresh frame.
fn click_text(h: &mut Harness, label: &str) {
    let list = h.frame();
    let p = list
        .cmds
        .iter()
        .find_map(|c| match c {
            DrawCmd::Text {
                pos, text, size, ..
            } if text == label => Some(Point::new(
                pos.x + text.chars().count() as f32 * size * 0.25,
                pos.y + size * 0.6,
            )),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{label} is not drawn"));
    h.input(InputEvent::PointerMove { pos: p });
    h.input(InputEvent::PointerDown {
        pos: p,
        button: PointerButton::Left,
    });
    h.input(InputEvent::PointerUp {
        pos: p,
        button: PointerButton::Left,
    });
}

/// Pump for a short while, so that a wrong outcome has the time to show.
/// Only for checks that something does not happen.
fn settle(h: &mut Harness) {
    for _ in 0..30 {
        h.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn outside_the_sandbox_nothing_changes() {
    let scene = Scene::new("sandbox-outside");
    let disk = TestDisk::new();
    disk.refuse_listing("photos");
    let mut h = scene.harness(Arc::clone(&disk), None);
    h.start();
    assert!(h.wait_shown(&scene.images[0]));
    assert!(
        h.wait_until(|a| a.count() == 1),
        "the image alone, as before"
    );
    settle(&mut h);
    assert_eq!(h.app().access_prompt(), None);
    assert!(!has_text(&h.frame(), ALLOW_MESSAGE));
    assert_eq!(
        h.act(Action::PromptResult {
            id: 1,
            accepted: true
        }),
        Handled::Ignored
    );
    assert!(lock(&h.dialogs).shown.is_empty(), "no folder picker");
    // The folder was listed once: nothing lists it again.
    let listings = disk
        .ops_on("photos")
        .iter()
        .filter(|op| **op == "list_dir")
        .count();
    assert_eq!(listings, 1);

    // A move keeps no grant either.
    let out = scene.root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    h.answer_dialog(vec![out.clone()]);
    h.act(Action::MoveTo);
    assert!(h.wait_until(|_| out.join("a.png").exists()));
    assert!(h.wait_until(|a| !a.moving()));
    settle(&mut h);
    assert!(
        !scene.grants().exists(),
        "no grants file outside the sandbox"
    );

    // Escape quits as always.
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app().exit_requested());
}

#[test]
fn a_refused_folder_is_asked_for_and_listed_once_allowed() {
    let scene = Scene::new("sandbox-allow");
    let disk = TestDisk::new();
    disk.refuse_listing("photos");
    let bookmarks = FakeBookmarks::default();
    let store = scene.store(bookmarks.clone());
    let mut h = scene.harness(Arc::clone(&disk), Some(store));
    h.start();
    assert!(h.wait_shown(&scene.images[0]));
    assert!(h.wait_until(|a| a.access_prompt().is_some()));
    assert_eq!(h.app().access_prompt(), Some(scene.photos.as_path()));
    assert_eq!(h.app().count(), 1, "the image alone until allowed");
    let list = h.frame();
    assert!(has_text(&list, ALLOW_MESSAGE));
    assert!(has_text(&list, ALLOW_BUTTON));

    // A folder that does not hold the image changes nothing.
    let elsewhere = scene.root.join("elsewhere");
    h.answer_dialog(vec![elsewhere]);
    click_text(&mut h, ALLOW_BUTTON);
    h.pump();
    match lock(&h.dialogs).shown.last() {
        Some(DialogKind::PickFolder { start_dir, title }) => {
            assert_eq!(start_dir.as_deref(), Some(scene.photos.as_path()));
            assert_eq!(title, ALLOW_TITLE);
        }
        other => panic!("expected the folder picker, got {other:?}"),
    }
    assert!(h.app().access_prompt().is_some());
    assert!(bookmarks.created().is_empty());
    assert!(has_text(&h.frame(), "Choose the folder of the image"));

    // The folder above it: the panel gives access, the folder lists at
    // once, and the grant is kept for later sessions.
    disk.allow_listing("photos");
    h.answer_dialog(vec![scene.root.path().to_path_buf()]);
    click_text(&mut h, ALLOW_BUTTON);
    assert!(h.wait_until(|a| a.count() == 3 && a.access_prompt().is_none()));
    let grants = scene.grants();
    assert!(h.wait_until(|_| grants.exists()));
    assert_eq!(bookmarks.created(), [scene.root.path().to_path_buf()]);
    assert!(!has_text(&h.frame(), ALLOW_MESSAGE));
    h.act(Action::Next);
    assert!(h.wait_shown(&scene.images[1]));
}

#[test]
fn a_grant_from_an_earlier_session_lists_the_folder_without_a_question() {
    let scene = Scene::new("sandbox-earlier");
    // The earlier session allowed the folder above the images.
    let earlier = scene.store(FakeBookmarks::default());
    earlier
        .with(|g| g.add(scene.root.path(), &StdDisk))
        .unwrap();
    drop(earlier);

    // This session: the first listing is refused while the grants still
    // load; once loaded, the folder is listed again and the question never
    // comes.
    let disk = TestDisk::new();
    disk.refuse_listing("photos");
    let gate = Gate::closed();
    let bookmarks = FakeBookmarks {
        gate: Some(Arc::clone(&gate)),
        disk: Some((Arc::clone(&disk), "photos")),
        ..FakeBookmarks::default()
    };
    let store = scene.store(bookmarks.clone());
    let mut h = scene.harness(Arc::clone(&disk), Some(store));
    h.start();
    assert!(h.wait_shown(&scene.images[0]));
    assert!(h.wait_until(|a| a.count() == 1));
    assert_eq!(h.app().access_prompt(), None);
    gate.open();
    assert!(h.wait_until(|a| a.count() == 3));
    assert_eq!(h.app().access_prompt(), None);
    assert!(
        bookmarks.created().is_empty(),
        "the grant was not made again"
    );
    let listings = disk
        .ops_on("photos")
        .iter()
        .filter(|op| **op == "list_dir")
        .count();
    assert_eq!(listings, 2);
}

#[test]
fn a_dismissed_folder_is_not_asked_for_again_in_the_session() {
    let scene = Scene::new("sandbox-dismiss");
    let disk = TestDisk::new();
    disk.refuse_listing("photos");
    let store = scene.store(FakeBookmarks::default());
    let mut h = scene.harness(Arc::clone(&disk), Some(store));
    h.start();
    assert!(h.wait_shown(&scene.images[0]));
    assert!(h.wait_until(|a| a.access_prompt().is_some()));
    h.frame();

    // Escape dismisses the prompt and does not quit.
    h.key(Key::Escape, Modifiers::NONE);
    assert_eq!(h.app().access_prompt(), None);
    assert!(!h.app().exit_requested());
    assert!(lock(&h.dialogs).shown.is_empty());

    // Another image of the same folder: still refused, not asked again.
    let now = h.now;
    h.app().open(scene.images[1].clone(), now);
    assert!(h.wait_shown(&scene.images[1]));
    assert!(h.wait_until(|a| a.count() == 1));
    settle(&mut h);
    assert_eq!(h.app().access_prompt(), None);
    assert!(!has_text(&h.frame(), ALLOW_MESSAGE));

    // The close mark dismisses too: another refused folder asks, and its
    // close mark ends the question.
    let other = scene.root.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let image = other.join("d.png");
    write_png(&image, &numbered(2, 2));
    disk.refuse_listing("other");
    let now = h.now;
    h.app().open(image.clone(), now);
    assert!(h.wait_until(|a| a.access_prompt() == Some(other.as_path())));
    let list = h.frame();
    let close = list
        .cmds
        .iter()
        .rev()
        .find_map(|c| match c {
            DrawCmd::Image { rect, texture, .. }
                if *texture == pandite_ui::Icon::Close.texture_id() =>
            {
                Some(rect.center())
            }
            _ => None,
        })
        .expect("close mark drawn");
    h.input(InputEvent::PointerMove { pos: close });
    h.input(InputEvent::PointerDown {
        pos: close,
        button: PointerButton::Left,
    });
    h.input(InputEvent::PointerUp {
        pos: close,
        button: PointerButton::Left,
    });
    assert_eq!(h.app().access_prompt(), None);
    // With no prompt, Escape quits again.
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app().exit_requested());
}

/// Let the clock pass the slow interval of a folder check `rounds` times,
/// ticking the core as the shell does, and let each look end.
fn time_passes(h: &mut Harness, rounds: usize) {
    for _ in 0..rounds {
        h.advance(SLOW_FOLDER_CHECK.as_millis() as u64);
        h.tick();
        assert!(h.app().library().wait_idle(support::WAIT));
        h.pump();
    }
}

#[test]
fn folder_checks_never_bring_the_question_back() {
    // A refused folder whose question the user dismissed: no look at the
    // folder, no question, no message.
    let scene = Scene::new("sandbox-watch");
    let disk = TestDisk::new();
    disk.refuse_listing("photos");
    let store = scene.store(FakeBookmarks::default());
    let mut h = scene.harness(Arc::clone(&disk), Some(store));
    h.start();
    assert!(h.wait_shown(&scene.images[0]));
    assert!(h.wait_until(|a| a.access_prompt().is_some()));
    h.frame();
    h.key(Key::Escape, Modifiers::NONE);
    assert_eq!(h.app().access_prompt(), None);
    assert_eq!(h.app().next_tick_at(), None);
    let wakes = h.wakes();
    time_passes(&mut h, 3);
    assert_eq!(h.app().access_prompt(), None);
    assert!(!has_text(&h.frame(), ALLOW_MESSAGE));
    assert_eq!(h.app().notice(), None);
    assert_eq!(disk.listings("photos"), 1);
    assert_eq!(h.wakes(), wakes);
    assert_eq!(h.app().count(), 1);

    // A folder that listed and is refused later (its access ended): each
    // look fails without a word, and the list stays.
    let scene = Scene::new("sandbox-watch-later");
    let disk = TestDisk::new();
    let store = scene.store(FakeBookmarks::default());
    let mut h = scene.harness(Arc::clone(&disk), Some(store));
    h.start();
    assert!(h.wait_shown(&scene.images[0]));
    assert!(h.wait_until(|a| a.count() == 3));
    // The thumbnails of the strip are made before the count starts.
    h.frame();
    assert!(h.app().library().wait_idle(support::WAIT));
    h.pump();
    h.frame();
    disk.refuse_listing("photos");
    let (wakes, redraws) = (h.wakes(), h.redraws());
    time_passes(&mut h, 3);
    assert_eq!(disk.listings("photos"), 4);
    assert_eq!(h.app().access_prompt(), None);
    assert_eq!(h.app().notice(), None);
    assert_eq!(h.app().count(), 3);
    assert_eq!((h.wakes(), h.redraws()), (wakes, redraws));
    assert!(!has_text(&h.frame(), ALLOW_MESSAGE));
}

#[test]
fn move_and_copy_folders_are_kept_as_grants() {
    let scene = Scene::new("sandbox-move");
    let disk = TestDisk::new();
    let bookmarks = FakeBookmarks::default();
    let store = scene.store(bookmarks.clone());
    let mut h = scene.harness(Arc::clone(&disk), Some(store));
    h.start();
    assert!(h.wait_shown(&scene.images[0]));
    assert!(h.wait_until(|a| a.count() == 3));
    assert_eq!(
        h.app().access_prompt(),
        None,
        "a listed folder asks nothing"
    );

    let out = scene.root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    h.answer_dialog(vec![out.clone()]);
    h.act(Action::MoveTo);
    assert!(h.wait_until(|a| !a.moving() && a.count() == 2));
    let grants = scene.grants();
    assert!(h.wait_until(|_| grants.exists()));
    assert!(h.wait_until(|_| !bookmarks.created().is_empty()));
    assert_eq!(bookmarks.created(), std::slice::from_ref(&out));

    // A second copy into the same folder: its grant covers it already.
    h.answer_dialog(vec![out.clone()]);
    h.act(Action::CopyTo);
    let copies = |dir: &Path| std::fs::read_dir(dir).map_or(0, |d| d.count());
    assert!(h.wait_until(|a| !a.copying_files() && copies(&out) == 2));
    settle(&mut h);
    assert_eq!(bookmarks.created(), [out]);
}
