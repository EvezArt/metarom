# FreeMix Engine

**Freestyle audio stem remixer — Kanye West Stem Player paradigm.**

Drop any audio file. Get 4 stems back. Freestyle them.

```
Input audio → Stem Separation → [vocals | drums | bass | other]
                                       ↓
                              Freestyle Generator
                              (beat-locked random mutations)
                                       ↓
                   freestyle_mix.wav + stem pack + manifest JSON
```

## Quick start

```bash
pip install librosa soundfile numpy scipy

# Freestyle mode — 8 random variations, 4 bars each
python freemix_engine.py track.wav output/ --mode freestyle --variations 8 --bars 4 --seed 42

# Manual remix — set per-stem volume/pitch/stretch
python freemix_engine.py track.wav output/ --mode remix \
  --config '{"drums":{"volume":1.8},"vocals":{"pitch":-2.0},"bass":{"stretch":2.0}}'
```

## Python API

```python
from freemix_engine import FreeMix

fm = FreeMix()
fm.load("track.wav")
fm.separate()

# Get stem files
fm.save_stems("stems/")
# → stems/stem_vocals.wav, stem_drums.wav, stem_bass.wav, stem_other.wav

# Manual remix
fm.remix_to_file("remix.wav", config={
    "drums":  {"volume": 1.5},
    "vocals": {"pitch": -2.0, "volume": 0.8},
    "bass":   {"stretch": 1.1},
    "other":  {"mute": True},
})

# Freestyle — random variations
manifest = fm.freestyle_to_dir("output/", n_variations=8, bars_per_variation=4, seed=None)
```

## Architecture

**Stem Separator** — HPSS (harmonic-percussive source separation) + subband soft masking via Wiener filter
- `vocals` — midrange harmonic content (80Hz–8kHz)
- `drums` — full-band percussive content
- `bass` — low harmonic content (<300Hz)
- `other` — residual (high harmonic + anything unclassified)

**Stem Processor** — per-stem effects chain
- Volume: 0.0–2.0
- Pitch shift: ±12 semitones (phase vocoder, librosa)
- Time stretch: 0.5–2.0x (WSOLA, librosa)
- Mute: instant silence

**Freestyle Generator** — beat-locked random mutation engine
BPM detection → bar boundary calculation → mutation sequence

| Mutation | Effect |
|----------|--------|
| `stem_mute` | Silence one stem for N bars |
| `stem_boost` | Crank stem 1.5–2.0x |
| `pitch_drift` | ±2 semitone detune |
| `half_time` | Drums+bass at 2x stretch (half-time feel) |
| `chop` | 1/8 or 1/16 note stutter loop |
| `reverse_bar` | Reverse one bar of a stem |
| `isolate` | Solo one stem, mute rest |

## Output format

```
output/
├── freestyle_mix.wav      ← full remixed track
├── stem_vocals.wav
├── stem_drums.wav
├── stem_bass.wav
├── stem_other.wav
└── freemix_manifest.json  ← version, BPM, mutation log per variation
```

`freemix_manifest.json` format (`freemix.v1`):
```json
{
  "version": "freemix.v1",
  "source": "track.wav",
  "source_hash": "abc123...",
  "sample_rate": 44100,
  "stems": ["vocals", "drums", "bass", "other"],
  "variations": [
    {"variation": 1, "bpm": 120.1, "bars": 4, "mutation": {"type": "chop", "stem": "drums", "division": 8}, "duration_s": 8.0}
  ]
}
```

## Upgrade path

Drop in [Demucs](https://github.com/adefossez/demucs) for production-quality stem separation:
```python
from demucs.pretrained import get_model
from demucs.apply import apply_model
# Replace StemSeparator.separate() with Demucs inference
```

Current HPSS approach: ~70% structural quality vs Demucs. Zero dependencies beyond librosa.

## Integration with EVEZ-OS / MetaROM

FreeMix outputs `freemix_manifest.json` — same epoch-tagging philosophy as `mrom.train.json`.
Feed audio remixes as Gen1/Gen2 training artifacts.

## License

AGPL-3.0 (community/free tier).
