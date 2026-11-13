//! G3: animation plays.

use super::*;
use crate::AnimationProvider;
use pandite_core::{Animation, Content, ContentKind, DisplayMode, Frame, LoopCount};
use std::sync::Arc;
use std::time::Duration;

const COLORS: [[u8; 3]; 3] = [[255, 0, 0], [0, 255, 0], [0, 0, 255]];
const DELAYS_MS: [u64; 3] = [100, 50, 200];

fn animation(loops: LoopCount) -> Animation {
    let frames = COLORS
        .iter()
        .zip(DELAYS_MS)
        .map(|(c, ms)| Frame {
            image: rgba8(8, 8, |_, _| [c[0], c[1], c[2], 255]),
            delay: Duration::from_millis(ms),
        })
        .collect();
    Animation::new(frames, loops).expect("animation")
}

fn view() -> ViewState {
    ViewState {
        mode: DisplayMode::Zoom(2.0),
        ..ViewState::default()
    }
}

fn shown(r: &mut Renderer) -> [u8; 3] {
    let px = frame(r, &view(), &DrawList::new());
    px.rgb(16, 16)
}

fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

#[test]
fn animation_advances_frames() {
    let gpu = gpu_or_skip!();
    let anim = animation(LoopCount::Infinite);
    // Both GPU strategies: every frame resident, and streaming through one
    // texture when the budget is too small.
    for cache_bytes in [crate::ANIMATION_CACHE_BYTES, 1] {
        let mut r = offscreen(&gpu, 32, 32, plain(Color::BLACK));
        r.providers_mut().register(
            ContentKind::Animation,
            Arc::new(AnimationProvider { cache_bytes }),
        );
        r.set_content(Content::Animation(anim.clone()))
            .expect("set animation");
        assert_eq!(r.content_kind(), Some(ContentKind::Animation));
        let mut elapsed = Duration::ZERO;
        let steps = [0, 60, 50, 39, 1, 100, 99, 1, 0, 120, 333, 1000, 7, 3];
        let mut seen = [false; 3];
        for step in steps {
            elapsed += ms(step);
            r.update(ms(step));
            let (index, _) = anim.frame_at(elapsed);
            seen[index] = true;
            assert_eq!(r.content_frame(), Some(index), "the frame on screen");
            assert_eq!(
                shown(&mut r),
                COLORS[index],
                "budget {cache_bytes}: at {elapsed:?} frame_at says {index}"
            );
        }
        assert_eq!(seen, [true; 3], "every frame was shown");
    }
}

#[test]
fn animation_reports_next_deadline() {
    let gpu = gpu_or_skip!();
    let anim = animation(LoopCount::Infinite);
    let mut r = offscreen(&gpu, 32, 32, plain(Color::BLACK));
    r.set_content(Content::Animation(anim.clone()))
        .expect("set animation");
    assert_eq!(r.update(Duration::ZERO), Some(ms(100)));
    assert_eq!(r.update(ms(30)), Some(ms(70)));
    assert_eq!(r.update(ms(70)), Some(ms(50)));
    assert_eq!(r.update(ms(49)), Some(ms(1)));
    assert_eq!(r.update(ms(1)), Some(ms(200)));
    // Across a loop boundary: 350 ms per loop, 360 ms is 10 ms into frame 0.
    assert_eq!(r.update(ms(210)), Some(ms(90)));
    let mut elapsed = ms(360);
    for step in [1, 17, 33, 250, 999] {
        elapsed += ms(step);
        assert_eq!(
            r.update(ms(step)),
            anim.frame_at(elapsed).1,
            "at {elapsed:?}"
        );
    }
    // The renderer draws the frame the deadline belongs to.
    assert_eq!(shown(&mut r), COLORS[anim.frame_at(elapsed).0]);

    // A still never asks for a redraw.
    r.set_content(Content::Still(rgba8(4, 4, |_, _| [1, 2, 3, 255])))
        .expect("set still");
    assert_eq!(r.update(ms(16)), None);
    // No content: nothing is due either.
    r.clear_content();
    assert_eq!(r.update(ms(16)), None);
}

#[test]
fn finite_animation_holds_last_frame() {
    let gpu = gpu_or_skip!();
    let anim = animation(LoopCount::Finite(2));
    let mut r = offscreen(&gpu, 32, 32, plain(Color::BLACK));
    r.set_content(Content::Animation(anim))
        .expect("set animation");
    assert_eq!(r.update(Duration::ZERO), Some(ms(100)));
    assert_eq!(shown(&mut r), COLORS[0]);
    // Second loop, last frame, 10 ms before the end of play.
    assert_eq!(r.update(ms(690)), Some(ms(10)));
    assert_eq!(shown(&mut r), COLORS[2]);
    // Two loops (700 ms) are over: the last frame stays and nothing is due.
    assert_eq!(r.update(ms(20)), None);
    assert_eq!(shown(&mut r), COLORS[2]);
    for _ in 0..3 {
        assert_eq!(r.update(ms(1000)), None);
        assert_eq!(shown(&mut r), COLORS[2]);
    }
    // Setting the content again restarts playback.
    let anim = animation(LoopCount::Finite(1));
    r.set_content(Content::Animation(anim))
        .expect("set animation");
    assert_eq!(r.update(Duration::ZERO), Some(ms(100)));
    assert_eq!(shown(&mut r), COLORS[0]);
}
