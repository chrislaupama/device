//! The single table mapping assistant highlight targets
//! (`saine_core::assistant::UI_TARGETS` and `track:<id>:<part>`) to window
//! rectangles in OUR layout (window points). It mirrors the geometry in
//! `ui/app.slint`; keep both in sync. Targets that are not on screen in the
//! current layout (hidden panel, scrolled-out track) return `None`, and the
//! UI falls back to showing only the note text.

/// Inputs that move things around.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub w: f32,
    pub h: f32,
    pub sounds: bool,
    pub mixer: bool,
    pub assistant: bool,
    /// Bottom panel shows the Clip tab (390 pt tall, no inspector).
    pub clip_tab: bool,
    pub scroll_y: f32,
}

pub type Rect = (f32, f32, f32, f32);

// Panel geometry (app.slint `AppWindow`).
pub const MIXER_BOTTOM_H: f32 = 390.0;
pub const DEFAULT_BOTTOM_H: f32 = 306.0;
pub const ASSISTANT_W: f32 = 252.5;
pub const ASSISTANT_GAP: f32 = 5.0;

impl Layout {
    pub fn bottom_h(&self) -> f32 {
        if self.mixer || self.clip_tab { MIXER_BOTTOM_H } else { DEFAULT_BOTTOM_H }
    }
    /// Height of the browser/timeline row (y 78 ..).
    pub fn top_h(&self) -> f32 {
        self.h - 88.0 - self.bottom_h()
    }
    pub fn bottom_y(&self) -> f32 {
        self.h - 5.0 - self.bottom_h()
    }
    pub fn timeline_x(&self) -> f32 {
        if self.sounds { 262.0 } else { 5.0 }
    }
    pub fn timeline_w(&self) -> f32 {
        let right = if self.assistant { ASSISTANT_W + ASSISTANT_GAP } else { 0.0 };
        self.w - self.timeline_x() - 5.0 - right
    }
}

/// Window rect of a highlight target. `track_index` resolves `track:<id>:…` ids.
pub fn target_rect(l: &Layout, target: &str, track_index: impl Fn(u64) -> Option<usize>) -> Option<Rect> {
    let w = l.w;
    let (tx, tw) = (l.timeline_x(), l.timeline_w());
    let by = l.bottom_y();
    let r = match target {
        // ---- header (x anchored left, toggles anchored right) ----
        "header.sounds_toggle" => (67.0, 39.0, 62.0, 25.0),
        "transport.rewind" => (368.0, 38.0, 28.0, 28.0),
        "transport.play" => (396.0, 38.0, 28.0, 28.0),
        "transport.record" => (423.0, 38.0, 28.0, 28.0),
        "transport.loop" => (470.0, 41.0, 22.0, 22.0),
        "transport.automation_arm" => (496.0, 41.0, 21.0, 22.0),
        "transport.bar" => (524.0, 42.0, 48.0, 20.0),
        "transport.time" => (592.0, 42.0, 88.0, 20.0),
        "transport.bpm" => (700.0, 42.0, 57.0, 20.0),
        "transport.tap" => (762.0, 44.0, 23.0, 15.0),
        "transport.key" => (809.0, 42.0, 86.0, 20.0),
        "transport.time_signature" => (913.0, 42.0, 62.0, 20.0),
        "transport.metronome" => (981.0, 41.0, 22.0, 22.0),
        "header.master_volume" => (1020.0, 41.0, 100.0, 22.0),
        "header.assistant_toggle" => (w - 147.0, 33.0, 25.0, 31.0),
        "header.mixer_toggle" => (w - 119.0, 31.0, 25.0, 33.0),
        "header.editor_toggle" => (w - 91.0, 39.0, 25.0, 25.0),
        "header.musical_typing" => (w - 63.0, 39.0, 25.0, 25.0),
        "header.settings" => (w - 36.0, 39.0, 25.0, 25.0),
        // ---- browser (only while visible) ----
        "browser.search" if l.sounds => (13.0, 82.0, 38.0, 19.0),
        "browser.library" if l.sounds => (53.0, 82.0, 38.0, 19.0),
        "browser.devices" if l.sounds => (92.0, 82.0, 42.0, 19.0),
        "browser.files" if l.sounds => (136.0, 82.0, 27.0, 19.0),
        // ---- timeline header (timeline-panel relative) ----
        "timeline.returns" => (tx + 57.0, 83.0, 62.0, 18.0),
        "timeline.master_toggle" => (tx + 123.0, 83.0, 61.0, 18.0),
        "timeline.overview" => (tx + 201.5, 83.0, (tw - 598.5).max(0.0), 17.0),
        "timeline.smart_fill" => (tx + tw - 388.0, 83.0, 67.0, 18.0),
        "timeline.automation_toggle" => (tx + tw - 317.0, 83.0, 18.0, 18.0),
        "timeline.draw_mode" => (tx + tw - 294.0, 83.0, 17.0, 18.0),
        "timeline.tool" => (tx + tw - 272.0, 83.0, 28.0, 18.0),
        "timeline.vertical_zoom" => (tx + tw - 242.0, 83.0, 95.0, 18.0),
        "timeline.horizontal_zoom" => (tx + tw - 149.0, 83.0, 94.0, 18.0),
        "timeline.snap" => (tx + tw - 56.0, 83.0, 49.0, 18.0),
        "timeline.add_track" => (tx, 105.5, 184.0, 39.0),
        "timeline.ruler" => (tx + 191.0, 105.5, tw - 191.0, 39.0),
        // ---- inspector + devices (bottom panel; hidden while the mixer is open) ----
        "inspector.input" if !l.mixer && !l.clip_tab => (15.0, by + 37.0, 149.0, 19.0),
        "inspector.output" if !l.mixer && !l.clip_tab => (15.0, by + 63.0, 149.0, 18.0),
        "inspector.trim" if !l.mixer && !l.clip_tab => (20.0, by + 120.0, 54.0, 68.0),
        "inspector.width" if !l.mixer && !l.clip_tab => (96.0, by + 120.0, 54.0, 68.0),
        "inspector.pan" if !l.mixer && !l.clip_tab => (20.0, by + 189.0, 54.0, 68.0),
        "inspector.mono" if !l.mixer && !l.clip_tab => (96.0, by + 189.0, 54.0, 68.0),
        "inspector.fader" if !l.mixer && !l.clip_tab => (188.0, by + 35.0, 63.0, 260.0),
        "devices.add_device" if !l.mixer && !l.clip_tab => (270.0, by + 34.5, 182.5, 264.0),
        t if t.starts_with("track:") => return track_rect(l, t, track_index),
        _ => return None,
    };
    Some(r)
}

fn track_rect(l: &Layout, t: &str, track_index: impl Fn(u64) -> Option<usize>) -> Option<Rect> {
    let mut it = t.split(':');
    let (_, id, part) = (it.next()?, it.next()?.parse::<u64>().ok()?, it.next()?);
    let i = track_index(id)?;
    let (tx, tw) = (l.timeline_x(), l.timeline_w());
    let y = 78.0 + 66.5 + i as f32 * 74.5 - l.scroll_y;
    // Visible track area: between the top "Add track" row and the master row.
    let (top, bottom) = (78.0 + 66.5, 78.0 + l.top_h() - 30.5);
    if y + 74.0 <= top || y >= bottom {
        return None;
    }
    let r = match part {
        "header" => (tx, y, 191.0, 74.0),
        "mute" => (tx + 28.0, y + 41.5, 11.0, 11.0),
        "solo" => (tx + 42.0, y + 41.5, 11.0, 11.0),
        "arm" => (tx + 55.0, y + 41.5, 11.0, 11.0),
        "pan" => (tx + 84.0, y + 33.5, 26.0, 26.0),
        "volume" => (tx + 116.0, y + 41.0, 68.0, 14.0),
        "lane" => (tx + 191.0, y, tw - 191.0, 74.0),
        _ => return None,
    };
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use saine_core::assistant::UI_TARGETS;

    fn default_layout() -> Layout {
        Layout { w: 1440.0, h: 820.0, sounds: true, mixer: false, assistant: false, clip_tab: false, scroll_y: 0.0 }
    }

    #[test]
    fn every_ui_target_maps_in_the_default_layout() {
        let l = default_layout();
        for t in UI_TARGETS {
            let r = target_rect(&l, t, |_| None).unwrap_or_else(|| panic!("{t} unmapped"));
            assert!(r.0 >= 0.0 && r.1 >= 0.0 && r.0 + r.2 <= 1440.0 && r.1 + r.3 <= 820.0, "{t} {r:?}");
        }
    }

    #[test]
    fn layout_dependent_targets() {
        let mut l = default_layout();
        assert_eq!(target_rect(&l, "transport.bpm", |_| None), Some((700.0, 42.0, 57.0, 20.0)));
        l.mixer = true;
        assert_eq!(target_rect(&l, "inspector.fader", |_| None), None);
        l.assistant = true;
        let ruler = target_rect(&l, "timeline.ruler", |_| None).unwrap();
        assert!((ruler.0 + ruler.2 - (1440.0 - 5.0 - 257.5)).abs() < 1e-3);
        assert_eq!(target_rect(&l, "track:7:volume", |id| (id == 7).then_some(0)), Some((262.0 + 116.0, 144.5 + 41.0, 68.0, 14.0)));
        assert_eq!(target_rect(&l, "track:7:volume", |_| None), None);
        l.scroll_y = 1000.0;
        assert_eq!(target_rect(&l, "track:7:header", |_| Some(0)), None, "scrolled out");
    }
}
