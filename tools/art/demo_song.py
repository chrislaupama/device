#!/usr/bin/env python3
"""Write the demo song used for app screenshots: a full arrangement, not a test state.

    python3 tools/art/demo_song.py "/tmp/Night Drive"
    device-studio --open "/tmp/Night Drive" --showcase arrange ...

The app titles a project after its folder, so keep the folder name "Night Drive".

Every sound is synthesised here (drums, vocal-like phrases, a riser), so the
project has no third-party media. Deterministic: same output every run.
"""
import json
import math
import random
import struct
import sys
import wave
from pathlib import Path

PPQ = 960
BAR = 4 * PPQ
BPM = 124.0
SR = 44100
rng = random.Random(7)

out = Path(sys.argv[1] if len(sys.argv) > 1 else "Night Drive").resolve()
(out / "media").mkdir(parents=True, exist_ok=True)

# ---------- audio ----------

def beats_to_frames(beats):
    return int(round(beats * 60.0 / BPM * SR))


def write_wav(name, chans):
    path = out / "media" / name
    n = len(chans[0])
    with wave.open(str(path), "wb") as w:
        w.setnchannels(len(chans))
        w.setsampwidth(2)
        w.setframerate(SR)
        frames = bytearray()
        for i in range(n):
            for c in chans:
                frames += struct.pack("<h", int(max(-1.0, min(1.0, c[i])) * 32000))
        w.writeframes(bytes(frames))
    return path, len(chans), n


def drums(bars, fill=False):
    n = beats_to_frames(4 * bars)
    l = [0.0] * n
    sixteenth = beats_to_frames(0.25)
    def add(at, sig, gain=1.0, pan=0.0):
        for i, v in enumerate(sig):
            if at + i < n:
                l[at + i] += v * gain
    kick = [math.sin(2 * math.pi * (50 + 120 * math.exp(-i / 900)) * i / SR) * math.exp(-i / 5200) for i in range(9000)]
    snare = [(rng.uniform(-1, 1) * 0.8 + 0.4 * math.sin(2 * math.pi * 190 * i / SR)) * math.exp(-i / 2600) for i in range(9000)]
    hat = [rng.uniform(-1, 1) * math.exp(-i / 500) for i in range(2500)]
    steps = 16 * bars
    for s in range(steps):
        at = s * sixteenth
        bar_step = s % 16
        if bar_step in (0, 6, 8, 11) or (bar_step == 14 and s // 16 % 2):
            add(at, kick, 0.95)
        if bar_step in (4, 12):
            add(at, snare, 0.75)
        if fill and s >= steps - 8 and s % 2 == 0:
            add(at, snare, 0.35 + 0.05 * (s - steps + 8))
        add(at, hat, 0.22 if s % 2 else 0.12)
    return [l]


def vocal(bars, phrases, base_hz):
    """Breathy, vibrato-laden phrases: reads as a vocal take on the timeline."""
    n = beats_to_frames(4 * bars)
    l = [0.0] * n
    for start_beat, len_beats, semis in phrases:
        a, b = beats_to_frames(start_beat), beats_to_frames(start_beat + len_beats)
        f = base_hz * 2 ** (semis / 12)
        ph = 0.0
        for i in range(a, min(b, n)):
            t = (i - a) / SR
            env = min(1.0, t / 0.04) * min(1.0, (b - i) / (0.12 * SR)) * (0.7 + 0.3 * math.sin(2 * math.pi * 1.3 * t))
            ph += 2 * math.pi * f * (1 + 0.012 * math.sin(2 * math.pi * 5.5 * t)) / SR
            s = sum(math.sin(k * ph) / k for k in (1, 2, 3, 5))
            l[i] = (0.42 * s + 0.08 * rng.uniform(-1, 1)) * env
    return [l]


def riser(bars):
    n = beats_to_frames(4 * bars)
    l, r = [0.0] * n, [0.0] * n
    lp = 0.0
    for i in range(n):
        x = i / n
        a = 0.02 + 0.6 * x * x
        lp += (0.02 + 0.5 * x) * (rng.uniform(-1, 1) - lp)
        s = lp * a * 1.6 + 0.25 * x * math.sin(2 * math.pi * (200 + 1800 * x * x) * i / SR)
        l[i], r[i] = s, s * (0.8 + 0.2 * math.sin(i / 3000))
    return [l, r]


# ---------- arrangement ----------

SECTIONS = [  # name, bars, palette colour
    ("Intro", 4, 0x68a8c0), ("Verse", 8, 0x60d890), ("Chorus", 8, 0xd85088),
    ("Break", 4, 0xd0a050), ("Chorus", 8, 0xd85088), ("Outro", 4, 0x9080d0),
]
starts = []
bar = 0
for _, n, _ in SECTIONS:
    starts.append(bar)
    bar += n
TOTAL_BARS = bar

next_id = 1
def nid():
    global next_id
    next_id += 1
    return next_id - 1

assets, tracks = [], []

def asset(path, chans, frames):
    a = {"absolute_path": str(path), "relative_path": f"media/{path.name}", "channels": chans,
         "frames": frames, "id": nid(), "sample_rate": float(SR)}
    assets.append(a)
    return a

def track(name, kind, color, gain=1.0, pan=0.0, height=0.5):
    t = {"id": nid(), "kind": kind, "name": name, "color": color, "gain": gain, "pan": pan, "height": height,
         "mute": False, "solo": False, "arm": False, "monitor": False, "input": None,
         "clips": [], "midi_clips": [], "devices": [], "automation": []}
    tracks.append(t)
    return t

def audio_clip(t, a, name, at_bar, bars=None):
    frames = a["frames"] if bars is None else min(a["frames"], beats_to_frames(4 * bars))
    t["clips"].append({"id": nid(), "asset": a["id"], "name": name, "start": at_bar * BAR,
                       "source_offset": 0, "source_length": frames, "gain": 1.0})

def midi_clip(t, name, at_bar, bars, pattern_bars, notes):
    """notes: (beat, length_beats, pitch, velocity) within one pattern, repeated to fill."""
    out_notes = []
    for rep in range(bars // pattern_bars):
        for beat, length, pitch, vel in notes:
            out_notes.append({"id": len(out_notes) + 1, "pitch": pitch,
                              "start": int((rep * pattern_bars * 4 + beat) * PPQ),
                              "length": int(length * PPQ), "velocity": vel})
    t["midi_clips"].append({"id": nid(), "name": name, "start": at_bar * BAR, "length": bars * BAR,
                            "notes": out_notes})

# Chords: Am - F - C - G, one bar each.
PROG = [(57, [57, 60, 64]), (53, [53, 57, 60]), (48, [52, 55, 60]), (55, [55, 59, 62])]

drum_t = track("Drums", "audio", 0, 0.9)
bass_t = track("Bass", "instrument", 7)
keys_t = track("Keys", "instrument", 5, 0.8, -0.15)
lead_t = track("Lead", "instrument", 2, 0.75, 0.2)
vox_t = track("Vocal", "audio", 9, 0.95)
fx_t = track("FX", "audio", 11, 0.6)

reverb_t = track("Reverb", "return", 6, 0.8, height=0.4)
delay_t = track("Delay", "return", 8, 0.7, height=0.4)


def device(t, builtin_id, params=None):
    t["devices"].append({"id": nid(), "enabled": True, "state": None, "params": params or {},
                         "plugin": {"format": "builtin", "id": builtin_id, "kind": "effect", "name": builtin_id,
                                    "path": None, "vendor": "Device", "version": ""}})


device(drum_t, "glue-compressor")
device(drum_t, "channeleq")
device(bass_t, "tapesaturation")
device(lead_t, "chorus")
device(vox_t, "pitch-corrector", {"key_follow": 0.0, "scale": 1.0, "speed": 100.0, "retune_speed_ms": 20.0, "humanize": 15.0})
device(vox_t, "tuner")
device(vox_t, "spectrum")
device(vox_t, "compressor")
device(fx_t, "auto-filter", {"frequency": 2400.0, "resonance": 0.3})
device(reverb_t, "reverb")
device(delay_t, "delay")
for t, rv, dl in ((keys_t, 0.4, 0.0), (lead_t, 0.3, 0.35), (vox_t, 0.45, 0.25)):
    t["sends"] = [{"target": reverb_t["id"], "level": rv}] + ([{"target": delay_t["id"], "level": dl}] if dl else [])

beat = asset(*write_wav("beat-2bar.wav", drums(2)))
fill = asset(*write_wav("beat-fill.wav", drums(2, fill=True)))
v_verse = asset(*write_wav("vocal-verse.wav", vocal(8, [(1, 2.5, 0), (4.5, 3, 3), (9, 2, 5), (12, 3.5, 3),
                                                         (17, 2.5, 0), (20.5, 3, -2), (25, 2, 0), (28, 3.5, 3)], 220)))
v_chorus = asset(*write_wav("vocal-chorus.wav", vocal(8, [(0, 3.5, 7), (4, 3.5, 5), (8, 3, 3), (12, 3.5, 0),
                                                           (16, 3.5, 7), (20, 3.5, 8), (24, 3, 7), (28, 3.5, 3)], 220)))
rise = asset(*write_wav("riser.wav", riser(4)))

intro, verse, ch1, brk, ch2, outro = starts
# Drums: two-bar loops, a fill before each chorus, out in the break.
for s, n in ((verse, 8), (ch1, 8), (ch2, 8), (outro, 4)):
    for b in range(s, s + n, 2):
        last = b + 2 == s + n and s in (verse,)
        audio_clip(drum_t, fill if last else beat, "Beat fill" if last else "Beat", b)
audio_clip(drum_t, beat, "Beat", intro + 2)

bass_line = [(0, 0.75, 33, 110), (1, 0.5, 33, 90), (1.5, 0.5, 45, 80), (2.5, 1, 33, 100),
             (4, 0.75, 29, 110), (5, 0.5, 29, 90), (5.5, 0.5, 41, 80), (6.5, 1, 29, 100),
             (8, 0.75, 36, 110), (9, 0.5, 36, 90), (9.5, 0.5, 48, 80), (10.5, 1, 36, 100),
             (12, 0.75, 31, 110), (13, 0.5, 31, 90), (13.5, 0.5, 43, 80), (14.5, 1, 38, 100)]
for s, n, name in ((verse, 8, "Bass verse"), (ch1, 8, "Bass chorus"), (ch2, 8, "Bass chorus"), (outro, 4, "Bass outro")):
    midi_clip(bass_t, name, s, n, 4, bass_line)

pads = [(i * 4, 3.9, p, 84) for i, (_, chord) in enumerate(PROG) for p in chord]
stabs = [(i * 4 + o, 0.4, p, 100) for i, (_, chord) in enumerate(PROG) for o in (0, 1.5, 2.5) for p in chord + [chord[0] + 12]]
midi_clip(keys_t, "Pads", intro, 4, 4, pads)
midi_clip(keys_t, "Pads", verse, 8, 4, pads)
midi_clip(keys_t, "Stabs", ch1, 8, 4, stabs)
midi_clip(keys_t, "Pads", brk, 4, 4, pads)
midi_clip(keys_t, "Stabs", ch2, 8, 4, stabs)
midi_clip(keys_t, "Pads", outro, 4, 4, pads)

hook = [(0, 0.5, 76, 110), (0.5, 0.5, 74, 90), (1, 1, 72, 100), (2.5, 0.5, 69, 90), (3, 1, 72, 105),
        (4, 0.5, 72, 100), (4.5, 0.5, 74, 90), (5, 1.5, 76, 110), (7, 0.5, 79, 95),
        (8, 0.5, 76, 110), (8.5, 0.5, 74, 90), (9, 1, 72, 100), (10.5, 1, 67, 90),
        (12, 1, 69, 100), (13, 0.5, 71, 90), (13.5, 2, 72, 110)]
midi_clip(lead_t, "Hook", ch1, 8, 4, hook)
midi_clip(lead_t, "Hook", ch2, 8, 4, hook)
midi_clip(lead_t, "Break lead", brk, 4, 4, [(n[0], n[1] * 2, n[2] - 12, 80) for n in hook[::3]])

audio_clip(vox_t, v_verse, "Vocal verse", verse)
audio_clip(vox_t, v_chorus, "Vocal chorus", ch1)
audio_clip(vox_t, v_chorus, "Vocal chorus", ch2)

audio_clip(fx_t, rise, "Riser", brk)
audio_clip(fx_t, rise, "Riser", verse + 4, 4)

# Automation: the lead's filter opens through each chorus; keys swell in the intro.
lead_t["automation"].append({"enabled": True, "target": {"kind": "synth", "param": "filter_cutoff"}, "points": [
    {"time": ch1 * BAR, "value": 600.0, "curve": 0.0}, {"time": (ch1 + 8) * BAR, "value": 6000.0, "curve": -0.4},
    {"time": brk * BAR, "value": 900.0, "curve": 0.0}, {"time": ch2 * BAR, "value": 900.0, "curve": 0.0},
    {"time": (ch2 + 8) * BAR, "value": 8000.0, "curve": -0.4}]})
keys_t["automation"].append({"enabled": True, "target": {"kind": "volume"}, "points": [
    {"time": 0, "value": -30.0, "curve": 0.0}, {"time": verse * BAR, "value": -3.0, "curve": -0.5},
    {"time": (outro + 4) * BAR, "value": -40.0, "curve": 0.4}]})

project = {
    "name": "Night Drive",
    "transport": {"bpm": BPM, "key": None, "loop_enabled": False, "loop_start": ch1 * BAR, "loop_end": (ch1 + 8) * BAR,
                  "time_signature": {"numerator": 4, "denominator": 4}},
    "master_gain": 0.9,
    "tracks": tracks,
    "assets": assets,
    "next_id": next_id,
    "master_automation": [],
    "sections": [{"id": nid(), "name": n, "color": c, "length": b * BAR} for n, b, c in SECTIONS],
}
project["next_id"] = next_id
env = {"format": "device-studio-project", "schema_version": 1, "saved_with": "tools/art/demo_song.py", "project": project}
(out / "project.json").write_text(json.dumps(env, indent=2))
print(f"wrote {out} ({TOTAL_BARS} bars, {len(tracks)} tracks)")
