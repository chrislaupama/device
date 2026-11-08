//! Assistant panel state and flow (the reference's AI panel, called
//! "Assistant" here). The provider call runs on a worker thread
//! (`assistant_provider`); its result comes back through the controller's
//! job channel and goes `parse_turn` → `plan(doc, mode, turn)` → (Assist)
//! `commit` (one undo step, `Origin::Assistant`) → `after_change()`.
//! Plan errors are shown as an error entry and nothing is executed.

use crate::controller::{Controller, JobResult};
use crate::highlight::{self, Layout};
use crate::{ChatEntry, HighlightRect};
use saine_core::assistant::{self, Mode};
use slint::{ModelRc, SharedString, VecModel};
use std::time::{Duration, Instant};

/// How long a highlight pulses.
pub const HIGHLIGHT_SECS: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    User = 0,
    Reply = 1,
    Error = 2,
    Pending = 3,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: EntryKind,
    /// User prompt, assistant explanation, or error message.
    pub text: String,
    /// Action log (plan summary lines, then highlight notes).
    pub lines: Vec<String>,
    pub secs: f32,
    pub expanded: bool,
}

#[derive(Debug, Clone)]
pub struct Highlight {
    pub target: String,
    pub note: String,
    pub until: Instant,
}

pub struct AssistantState {
    pub mode: Mode,
    pub entries: Vec<Entry>,
    pub notice_visible: bool,
    /// Request in flight (start time). One at a time.
    pub busy: Option<Instant>,
    pub highlights: Vec<Highlight>,
    /// The user removed the selection from the chat context (until it changes).
    pub context_removed: Option<(Option<u64>, Option<u64>)>,
    /// Bumped when entries change, so the Slint model is rebuilt only then.
    pub rev: u64,
}

impl Default for AssistantState {
    fn default() -> Self {
        AssistantState { mode: Mode::Assist, entries: Vec::new(), notice_visible: true, busy: None, highlights: Vec::new(), context_removed: None, rev: 0 }
    }
}

impl AssistantState {
    pub fn last(&self) -> Option<&Entry> {
        self.entries.iter().rev().find(|e| e.kind != EntryKind::Pending)
    }

    fn push(&mut self, e: Entry) {
        self.entries.retain(|e| e.kind != EntryKind::Pending);
        self.entries.push(e);
        self.rev += 1;
    }

    pub fn to_model(&self) -> Vec<ChatEntry> {
        self.entries
            .iter()
            .map(|e| ChatEntry {
                kind: e.kind as i32,
                text: e.text.clone().into(),
                lines: ModelRc::new(VecModel::from(e.lines.iter().map(|l| SharedString::from(l.as_str())).collect::<Vec<_>>())),
                worked: format!("Worked for {:.1}s", e.secs).into(),
                expanded: e.expanded,
            })
            .collect()
    }

    /// (user prompt, reply text) pairs for the model's conversation history.
    fn history(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut it = self.entries.iter().peekable();
        while let Some(e) = it.next() {
            if e.kind == EntryKind::User
                && let Some(next) = it.peek()
                && next.kind == EntryKind::Reply
            {
                let mut reply = next.lines.join("\n");
                if !next.text.is_empty() {
                    reply = format!("{}\n{reply}", next.text);
                }
                out.push((e.text.clone(), reply.trim().to_string()));
            }
        }
        // Keep the request small: last 6 exchanges.
        let n = out.len().saturating_sub(6);
        out.split_off(n)
    }
}

impl Controller {
    pub fn toggle_assistant(&mut self) {
        self.assistant_open = !self.assistant_open;
        self.ui_dirty = true;
    }

    pub fn assistant_set_mode(&mut self, teach: bool) {
        self.assistant.mode = if teach { Mode::Teach } else { Mode::Assist };
        self.ui_dirty = true;
    }

    pub fn assistant_new_chat(&mut self) {
        // A request in flight is not cancelled; its result is dropped (busy cleared).
        self.assistant.entries.clear();
        self.assistant.busy = None;
        self.assistant.highlights.clear();
        self.assistant.rev += 1;
        self.ui_dirty = true;
    }

    pub fn assistant_dismiss_notice(&mut self) {
        self.assistant.notice_visible = false;
        self.ui_dirty = true;
    }

    pub fn assistant_toggle_entry(&mut self, i: usize) {
        if let Some(e) = self.assistant.entries.get_mut(i) {
            e.expanded = !e.expanded;
            self.assistant.rev += 1;
            self.ui_dirty = true;
        }
    }

    pub fn assistant_remove_context(&mut self) {
        self.assistant.context_removed = Some(self.selection_key());
        self.ui_dirty = true;
    }

    fn selection_key(&self) -> (Option<u64>, Option<u64>) {
        (self.selected_track.map(|t| t.0), self.selected_clip.map(|c| c.0))
    }

    /// Chat context chip: (name, scope label, track colour index, instrument), if any.
    pub fn assistant_context(&self) -> Option<(String, &'static str, saine_core::ColorIndex, bool)> {
        if self.assistant.context_removed == Some(self.selection_key()) {
            return None;
        }
        let p = self.project();
        if let Some(c) = self.selected_clip
            && let Some((ti, ci)) = p.find_clip(c)
        {
            let t = &p.tracks[ti];
            return Some((t.clips[ci].name.clone(), "Clip", t.color, t.kind == saine_core::TrackKind::Instrument));
        }
        let t = p.track(self.selected_track?)?;
        Some((t.name.clone(), "Track", t.color, t.kind == saine_core::TrackKind::Instrument))
    }

    fn selection_description(&self) -> Option<String> {
        self.assistant_context()?;
        let p = self.project();
        if let Some(c) = self.selected_clip
            && let Some((ti, ci)) = p.find_clip(c)
        {
            let t = &p.tracks[ti];
            return Some(format!("clip \"{}\" (id {}) on track {} \"{}\" (id {})", t.clips[ci].name, c.0, ti + 1, t.name, t.id.0));
        }
        let id = self.selected_track?;
        let i = p.track_index(id)?;
        Some(format!("track {} \"{}\" (id {})", i + 1, p.tracks[i].name, id.0))
    }

    /// Send a prompt. Never blocks: the provider runs on a worker thread.
    pub fn assistant_send(&mut self, text: &str) -> bool {
        let prompt = text.trim().to_string();
        if prompt.is_empty() || self.assistant.busy.is_some() {
            return false;
        }
        if !self.provider.available() {
            self.set_status("Set ANTHROPIC_API_KEY to enable the assistant", 4);
            return false;
        }
        let mut state = assistant::describe_state(self.project());
        state["next_id_hint"] = serde_json::json!(self.project().next_id);
        let req = crate::assistant_provider::Request { mode: self.assistant.mode, prompt: prompt.clone(), state, selection: self.selection_description(), history: self.assistant.history() };
        self.assistant.push(Entry { kind: EntryKind::User, text: prompt, lines: Vec::new(), secs: 0.0, expanded: true });
        self.assistant.entries.push(Entry { kind: EntryKind::Pending, text: "Working…".into(), lines: Vec::new(), secs: 0.0, expanded: true });
        self.assistant.rev += 1;
        let started = Instant::now();
        self.assistant.busy = Some(started);
        let provider = self.provider.clone();
        let tx = self.jobs_tx.clone();
        let mode = self.assistant.mode;
        self.pending_jobs += 1;
        let spawned = std::thread::Builder::new().name("assistant".into()).spawn(move || {
            let result = provider.complete(&req);
            let _ = tx.send(JobResult::Assistant { started, mode, result });
        });
        if let Err(e) = spawned {
            self.pending_jobs -= 1;
            self.assistant.busy = None;
            self.assistant.push(Entry { kind: EntryKind::Error, text: format!("Could not start the request: {e}"), lines: Vec::new(), secs: 0.0, expanded: true });
        }
        self.ui_dirty = true;
        true
    }

    /// Called from `poll_jobs` on the main thread.
    pub fn finish_assistant(&mut self, started: Instant, mode: Mode, result: Result<String, String>) {
        if self.assistant.busy != Some(started) {
            return; // superseded by "New chat"
        }
        self.assistant.busy = None;
        let secs = started.elapsed().as_secs_f32();
        self.ui_dirty = true;
        let error = |s: &mut Self, msg: String| {
            s.assistant.push(Entry { kind: EntryKind::Error, text: msg, lines: Vec::new(), secs, expanded: true });
        };
        let raw = match result {
            Ok(r) => r,
            Err(e) => return error(self, e),
        };
        let turn = match assistant::parse_turn(&raw) {
            Ok(t) => t,
            Err(e) => return error(self, format!("Couldn't use the reply: {e}")),
        };
        let plan = match assistant::plan(&self.doc, mode, &turn) {
            Ok(p) => p,
            Err(e) => return error(self, format!("Nothing was changed: {e}.")),
        };
        if mode == Mode::Assist
            && plan.batch.is_some()
        {
            if let Err(e) = assistant::commit(&mut self.doc, &plan) {
                return error(self, format!("Nothing was changed: {e}."));
            }
            self.after_change();
        }
        let until = Instant::now() + Duration::from_secs_f32(HIGHLIGHT_SECS);
        self.assistant.highlights = plan.highlights.iter().map(|(t, n)| Highlight { target: t.clone(), note: n.clone(), until }).collect();
        let mut lines = plan.summary.clone();
        lines.extend(plan.highlights.iter().map(|(t, n)| if n.is_empty() { format!("Look at {}.", target_label(t)) } else { format!("{}: {n}", target_label(t)) }));
        let text = plan.explanations.join("\n\n");
        if text.is_empty() && lines.is_empty() {
            lines.push("No changes were needed.".into());
        }
        self.assistant.push(Entry { kind: EntryKind::Reply, text, lines, secs, expanded: true });
    }

    pub fn highlight_layout(&self, w: f32, h: f32, sounds: bool) -> Layout {
        Layout { w, h, sounds, mixer: self.mixer_open, assistant: self.assistant_open, scroll_y: self.view.scroll_y as f32 }
    }

    /// Current highlight rects (expired ones dropped). Unknown-to-layout targets are skipped.
    pub fn highlight_rects(&mut self, layout: &Layout) -> Vec<HighlightRect> {
        let now = Instant::now();
        let before = self.assistant.highlights.len();
        self.assistant.highlights.retain(|h| h.until > now);
        if self.assistant.highlights.len() != before {
            self.ui_dirty = true;
        }
        let p = self.project();
        self.assistant
            .highlights
            .iter()
            .filter_map(|h| {
                let r = highlight::target_rect(layout, &h.target, |id| p.track_index(saine_core::TrackId(id)))?;
                Some(HighlightRect { x: r.0, y: r.1, w: r.2, h: r.3, note: h.note.clone().into() })
            })
            .collect()
    }
}

/// Friendly name for a target id in the action log.
pub fn target_label(t: &str) -> String {
    let named = match t {
        "transport.bpm" => "Tempo (BPM)",
        "transport.play" => "Play",
        "transport.loop" => "Loop",
        "transport.metronome" => "Metronome",
        "transport.time_signature" => "Time signature",
        "header.master_volume" => "Master volume",
        "header.mixer_toggle" => "Mixer button",
        "timeline.add_track" => "Add track",
        "inspector.fader" => "Track fader",
        _ => "",
    };
    if !named.is_empty() {
        return named.into();
    }
    if let Some(rest) = t.strip_prefix("track:") {
        let part = rest.rsplit(':').next().unwrap_or("");
        return format!("Track {part}");
    }
    let leaf = t.rsplit('.').next().unwrap_or(t).replace('_', " ");
    let mut c = leaf.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels() {
        assert_eq!(target_label("transport.bpm"), "Tempo (BPM)");
        assert_eq!(target_label("timeline.smart_fill"), "Smart fill");
        assert_eq!(target_label("track:3:volume"), "Track volume");
    }
    #[test]
    fn history_pairs_user_and_reply() {
        let mut s = AssistantState::default();
        s.push(Entry { kind: EntryKind::User, text: "a".into(), lines: vec![], secs: 0.0, expanded: true });
        s.push(Entry { kind: EntryKind::Reply, text: "".into(), lines: vec!["Did x.".into()], secs: 1.0, expanded: true });
        s.push(Entry { kind: EntryKind::User, text: "b".into(), lines: vec![], secs: 0.0, expanded: true });
        s.push(Entry { kind: EntryKind::Error, text: "e".into(), lines: vec![], secs: 0.0, expanded: true });
        assert_eq!(s.history(), vec![("a".to_string(), "Did x.".to_string())]);
    }
}
