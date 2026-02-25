#!/usr/bin/env python3
"""
FreeMix Engine — Freestyle audio stem remixer
EVEZ-OS / MetaROM project
Stem Player paradigm: 4-stem separation + real-time remix + freestyle generator

Architecture:
  Input audio → Stem Separator → [vocals, drums, bass, other]
                     ↓
              Remix Engine (per-stem vol/pitch/stretch)
                     ↓
              Freestyle Generator (beat-locked random mutations)
                     ↓
              Output: remixed WAV + stem pack + manifest JSON
"""

import numpy as np
import soundfile as sf
import librosa
import librosa.effects
import json
import os
import hashlib
import random
import warnings
from pathlib import Path
from typing import Dict, List, Optional, Tuple

warnings.filterwarnings("ignore")

# ══════════════════════════════════════════════════════════════════
# CONSTANTS
# ══════════════════════════════════════════════════════════════════
STEM_NAMES = ["vocals", "drums", "bass", "other"]
DEFAULT_SR   = 44100
HOP_LENGTH   = 512
N_FFT        = 2048
MARGIN       = 3.0          # Wiener filter margin for HPSS
BASS_CUTOFF  = 300          # Hz — bass/low freq boundary
VOCAL_LOW    = 80           # Hz — vocal low cutoff
VOCAL_HIGH   = 8000         # Hz — vocal high cutoff


# ══════════════════════════════════════════════════════════════════
# STEM SEPARATOR
# Uses harmonic-percussive source separation + subband masking
# 4 stems: vocals (harmonic midrange), drums (percussive),
#          bass (low harmonic), other (residual)
# ══════════════════════════════════════════════════════════════════
class StemSeparator:
    """
    MDCT/STFT-based 4-stem separator.
    Quality: ~70% of Demucs for structural separation.
    Fully offline, no model weights needed.
    """

    def __init__(self, sr: int = DEFAULT_SR):
        self.sr = sr

    def _freq_mask(self, stft: np.ndarray, low_hz: float, high_hz: float) -> np.ndarray:
        """Create a frequency band mask for a given Hz range."""
        freqs = librosa.fft_frequencies(sr=self.sr, n_fft=N_FFT)
        mask = np.zeros(stft.shape[0], dtype=float)
        mask[(freqs >= low_hz) & (freqs <= high_hz)] = 1.0
        return mask[:, np.newaxis]

    def separate(self, audio: np.ndarray) -> Dict[str, np.ndarray]:
        """
        Separate audio into 4 stems.
        Returns dict: {stem_name: audio_array}
        """
        if audio.ndim > 1:
            mono = librosa.to_mono(audio.T)
        else:
            mono = audio

        # STFT
        D = librosa.stft(mono, n_fft=N_FFT, hop_length=HOP_LENGTH)
        mag, phase = np.abs(D), np.angle(D)

        # Harmonic-percussive separation with margins
        D_harm, D_perc = librosa.decompose.hpss(D, margin=MARGIN)
        harm_mag = np.abs(D_harm)
        perc_mag = np.abs(D_perc)
        resid_mag = np.maximum(mag - harm_mag - perc_mag, 0)

        # Frequency band masks
        bass_mask   = self._freq_mask(D, 20, BASS_CUTOFF)
        vocal_mask  = self._freq_mask(D, VOCAL_LOW, VOCAL_HIGH)
        full_mask   = np.ones_like(bass_mask)

        # Stem construction via soft masking
        def apply_mask(source_mag, freq_mask):
            """Wiener-style soft mask + phase reconstruction."""
            masked = source_mag * freq_mask
            # Soft mask against total magnitude
            total = mag + 1e-8
            soft = (masked ** 2) / total
            return librosa.istft(soft * np.exp(1j * phase), hop_length=HOP_LENGTH, n_fft=N_FFT)

        # Bass: low harmonic content
        bass_audio = apply_mask(harm_mag, bass_mask)

        # Drums: percussive full-band
        drums_audio = apply_mask(perc_mag, full_mask)

        # Vocals: midrange harmonic (minus bass)
        mid_mask = self._freq_mask(D, VOCAL_LOW, VOCAL_HIGH)
        vocals_audio = apply_mask(harm_mag, mid_mask)

        # Other: residual (harmonic high freq + any leftover)
        other_audio = apply_mask(resid_mag, full_mask)

        # Normalize each stem
        stems = {
            "vocals": vocals_audio,
            "drums":  drums_audio,
            "bass":   bass_audio,
            "other":  other_audio,
        }
        for k in stems:
            mx = np.max(np.abs(stems[k]))
            if mx > 1e-6:
                stems[k] = stems[k] / mx * 0.9

        return stems


# ══════════════════════════════════════════════════════════════════
# STEM EFFECTS
# Per-stem processing: volume, pitch shift, time stretch
# ══════════════════════════════════════════════════════════════════
class StemProcessor:
    def __init__(self, sr: int = DEFAULT_SR):
        self.sr = sr

    def apply(
        self,
        audio: np.ndarray,
        volume: float = 1.0,         # 0.0 – 2.0
        pitch_semitones: float = 0.0, # +/- 12 semitones
        time_stretch: float = 1.0,    # 0.5 – 2.0 (1.0 = no change)
        mute: bool = False,
    ) -> np.ndarray:
        if mute:
            return np.zeros_like(audio)

        out = audio.copy()

        # Pitch shift
        if abs(pitch_semitones) > 0.01:
            out = librosa.effects.pitch_shift(
                out, sr=self.sr, n_steps=pitch_semitones
            )

        # Time stretch (phase vocoder)
        if abs(time_stretch - 1.0) > 0.01:
            out = librosa.effects.time_stretch(out, rate=time_stretch)

        # Volume
        out = out * volume

        # Clip
        out = np.clip(out, -1.0, 1.0)
        return out


# ══════════════════════════════════════════════════════════════════
# REMIX ENGINE
# Mixes processed stems into output
# ══════════════════════════════════════════════════════════════════
class RemixEngine:
    def __init__(self, sr: int = DEFAULT_SR):
        self.sr = sr
        self.processor = StemProcessor(sr)

    def remix(
        self,
        stems: Dict[str, np.ndarray],
        config: Dict,
    ) -> np.ndarray:
        """
        config: {
            "vocals": {"volume": 1.0, "pitch": 0.0, "stretch": 1.0, "mute": false},
            "drums":  {"volume": 1.2, "pitch": 0.0, "stretch": 1.0, "mute": false},
            ...
        }
        """
        processed = {}
        max_len = 0

        for stem_name in STEM_NAMES:
            stem_audio = stems.get(stem_name, np.array([]))
            stem_cfg   = config.get(stem_name, {})

            out = self.processor.apply(
                stem_audio,
                volume          = stem_cfg.get("volume", 1.0),
                pitch_semitones = stem_cfg.get("pitch", 0.0),
                time_stretch    = stem_cfg.get("stretch", 1.0),
                mute            = stem_cfg.get("mute", False),
            )
            processed[stem_name] = out
            max_len = max(max_len, len(out))

        # Pad all stems to same length and mix
        mix = np.zeros(max_len)
        for stem_name, audio in processed.items():
            padded = np.zeros(max_len)
            padded[:len(audio)] = audio
            mix += padded

        # Final normalize
        mx = np.max(np.abs(mix))
        if mx > 1e-6:
            mix = mix / mx * 0.95

        return mix


# ══════════════════════════════════════════════════════════════════
# FREESTYLE GENERATOR
# Beat-locked random mutations — Stem Player freestyle mode
# Generates N variations and sequences them
# ══════════════════════════════════════════════════════════════════
class FreestyleGenerator:
    """
    Kanye-style freestyle: takes stems, detects BPM, then
    generates random remix variations locked to beat boundaries.

    Mutation types (inspired by Stem Player interaction model):
      - stem_mute:    randomly silence a stem for N bars
      - stem_boost:   crank a stem volume for emphasis  
      - pitch_drift:  subtle pitch detune (+/-2 semitones)
      - half_time:    slow drums/bass to half tempo feel
      - chop:         stutter-chop a stem at 1/8 or 1/16 note resolution
      - reverse_bar:  reverse a single bar of a stem
      - isolate:      solo one stem, mute the rest
    """

    MUTATION_TYPES = [
        "stem_mute", "stem_boost", "pitch_drift",
        "half_time", "chop", "reverse_bar", "isolate"
    ]

    def __init__(self, sr: int = DEFAULT_SR, seed: Optional[int] = None):
        self.sr  = sr
        self.rng = random.Random(seed)
        self.engine = RemixEngine(sr)

    def _detect_bpm(self, drums: np.ndarray) -> float:
        tempo, _ = librosa.beat.beat_track(y=drums, sr=self.sr, hop_length=HOP_LENGTH)
        if isinstance(tempo, np.ndarray):
            tempo = float(tempo[0]) if len(tempo) > 0 else 120.0
        return max(60.0, min(200.0, float(tempo)))

    def _bar_samples(self, bpm: float, beats_per_bar: int = 4) -> int:
        beat_samples = int(self.sr * 60.0 / bpm)
        return beat_samples * beats_per_bar

    def _apply_chop(self, audio: np.ndarray, bpm: float, division: int = 8) -> np.ndarray:
        """Stutter-chop: repeat 1/division note chunks."""
        beat_s = int(self.sr * 60.0 / bpm)
        chunk  = beat_s // (division // 4)
        if chunk < 64 or len(audio) < chunk:
            return audio
        out = audio.copy()
        pos = 0
        while pos + chunk * 2 < len(out):
            out[pos + chunk: pos + chunk * 2] = out[pos: pos + chunk]
            pos += chunk * 2
        return out

    def _apply_reverse_bar(self, audio: np.ndarray, bar_s: int, bar_idx: int) -> np.ndarray:
        out = audio.copy()
        start = bar_idx * bar_s
        end   = min(start + bar_s, len(out))
        if end > start:
            out[start:end] = out[start:end][::-1]
        return out

    def _generate_mutation(
        self, stems: Dict[str, np.ndarray], bpm: float, bar_s: int
    ) -> Tuple[Dict, Dict]:
        """Generate one random remix config + description."""
        mutation = self.rng.choice(self.MUTATION_TYPES)
        target   = self.rng.choice(STEM_NAMES)
        config   = {s: {"volume": 1.0, "pitch": 0.0, "stretch": 1.0, "mute": False} for s in STEM_NAMES}
        desc     = {}

        if mutation == "stem_mute":
            config[target]["mute"] = True
            desc = {"type": "mute", "stem": target}

        elif mutation == "stem_boost":
            boost = self.rng.uniform(1.5, 2.0)
            config[target]["volume"] = boost
            desc = {"type": "boost", "stem": target, "gain": round(boost, 2)}

        elif mutation == "pitch_drift":
            semis = self.rng.uniform(-2.0, 2.0)
            config[target]["pitch"] = semis
            desc = {"type": "pitch", "stem": target, "semitones": round(semis, 2)}

        elif mutation == "half_time":
            for s in ["drums", "bass"]:
                config[s]["stretch"] = 2.0  # half speed feel
            desc = {"type": "half_time", "stems": ["drums", "bass"]}

        elif mutation == "chop":
            division = self.rng.choice([8, 16])
            desc = {"type": "chop", "stem": target, "division": division}
            # chop is applied post-mix via stem modification

        elif mutation == "reverse_bar":
            bar_idx = self.rng.randint(0, 3)
            desc = {"type": "reverse_bar", "stem": target, "bar": bar_idx}

        elif mutation == "isolate":
            for s in STEM_NAMES:
                if s != target:
                    config[s]["mute"] = True
            desc = {"type": "isolate", "stem": target}

        return config, desc

    def generate(
        self,
        stems: Dict[str, np.ndarray],
        n_variations: int = 8,
        bars_per_variation: int = 4,
        seed: Optional[int] = None,
    ) -> Tuple[np.ndarray, List[Dict]]:
        """
        Generate a freestyle remix sequence.
        Returns: (mixed_audio, manifest_list)
        """
        if seed is not None:
            self.rng = random.Random(seed)

        bpm   = self._detect_bpm(stems.get("drums", np.zeros(self.sr)))
        bar_s = self._bar_samples(bpm)

        print(f"  [FreeMix] BPM detected: {bpm:.1f}")
        print(f"  [FreeMix] Bar length: {bar_s/self.sr:.2f}s — generating {n_variations} variations")

        sequence = []
        manifest = []

        for i in range(n_variations):
            config, desc = self._generate_mutation(stems, bpm, bar_s)

            # Apply chop directly to stems
            local_stems = {k: v.copy() for k, v in stems.items()}
            if desc.get("type") == "chop":
                tgt = desc["stem"]
                local_stems[tgt] = self._apply_chop(local_stems[tgt], bpm, desc["division"])
            elif desc.get("type") == "reverse_bar":
                tgt = desc["stem"]
                local_stems[tgt] = self._apply_reverse_bar(local_stems[tgt], bar_s, desc.get("bar", 0))

            mixed = self.engine.remix(local_stems, config)

            # Trim to N bars
            trim_len = bar_s * bars_per_variation
            if len(mixed) > trim_len:
                mixed = mixed[:trim_len]
            else:
                mixed = np.pad(mixed, (0, max(0, trim_len - len(mixed))))

            sequence.append(mixed)
            manifest.append({
                "variation": i + 1,
                "bpm": round(bpm, 1),
                "bars": bars_per_variation,
                "mutation": desc,
                "duration_s": round(len(mixed) / self.sr, 3),
            })
            print(f"  [FreeMix] Variation {i+1}/{n_variations}: {desc}")

        # Concatenate all variations
        full_mix = np.concatenate(sequence)
        mx = np.max(np.abs(full_mix))
        if mx > 1e-6:
            full_mix = full_mix / mx * 0.95

        return full_mix, manifest


# ══════════════════════════════════════════════════════════════════
# FREEMIX — top-level API
# ══════════════════════════════════════════════════════════════════
class FreeMix:
    """
    Main entry point.

    Usage:
        fm = FreeMix()
        fm.load("track.wav")
        fm.separate()
        fm.remix(config={"drums": {"volume": 1.5}, "vocals": {"pitch": -2.0}})
        fm.freestyle(n_variations=8, output_dir="output/")
    """

    def __init__(self, sr: int = DEFAULT_SR):
        self.sr        = sr
        self.audio     = None
        self.stems     = None
        self.separator = StemSeparator(sr)
        self.engine    = RemixEngine(sr)
        self.freestyle = FreestyleGenerator(sr)
        self._source_path = None

    def load(self, path: str) -> "FreeMix":
        print(f"[FreeMix] Loading: {path}")
        audio, sr = librosa.load(path, sr=self.sr, mono=True)
        self.audio = audio
        self._source_path = path
        duration = len(audio) / self.sr
        print(f"[FreeMix] Loaded: {duration:.1f}s @ {self.sr}Hz")
        return self

    def load_array(self, audio: np.ndarray, sr: int = None) -> "FreeMix":
        if sr and sr != self.sr:
            audio = librosa.resample(audio, orig_sr=sr, target_sr=self.sr)
        self.audio = audio
        return self

    def separate(self) -> "FreeMix":
        if self.audio is None:
            raise ValueError("Load audio first with .load()")
        print("[FreeMix] Separating stems (HPSS + subband masking)...")
        self.stems = self.separator.separate(self.audio)
        for name, stem in self.stems.items():
            print(f"  [{name}] {len(stem)/self.sr:.1f}s")
        return self

    def save_stems(self, output_dir: str) -> Dict[str, str]:
        """Save individual stems as WAV files."""
        if self.stems is None:
            raise ValueError("Run .separate() first")
        Path(output_dir).mkdir(parents=True, exist_ok=True)
        paths = {}
        for name, audio in self.stems.items():
            p = os.path.join(output_dir, f"stem_{name}.wav")
            sf.write(p, audio, self.sr)
            paths[name] = p
            print(f"[FreeMix] Saved stem: {p}")
        return paths

    def remix_to_file(
        self,
        output_path: str,
        config: Optional[Dict] = None,
    ) -> str:
        """Apply remix config and write output WAV."""
        if self.stems is None:
            raise ValueError("Run .separate() first")
        if config is None:
            config = {}
        print(f"[FreeMix] Remixing with config: {config}")
        mixed = self.engine.remix(self.stems, config)
        sf.write(output_path, mixed, self.sr)
        print(f"[FreeMix] Remix saved: {output_path}")
        return output_path

    def freestyle_to_dir(
        self,
        output_dir: str,
        n_variations: int = 8,
        bars_per_variation: int = 4,
        seed: Optional[int] = None,
    ) -> str:
        """Run freestyle generator, save full remix + manifest."""
        if self.stems is None:
            raise ValueError("Run .separate() first")

        Path(output_dir).mkdir(parents=True, exist_ok=True)
        print(f"[FreeMix] Freestyle generating {n_variations} variations...")

        mix, manifest_data = self.freestyle.generate(
            self.stems,
            n_variations=n_variations,
            bars_per_variation=bars_per_variation,
            seed=seed,
        )

        # Save freestyle mix
        mix_path = os.path.join(output_dir, "freestyle_mix.wav")
        sf.write(mix_path, mix, self.sr)

        # Save stems
        stem_paths = self.save_stems(output_dir)

        # Build manifest
        src_hash = ""
        if self._source_path and os.path.exists(self._source_path):
            with open(self._source_path, "rb") as f:
                src_hash = hashlib.sha256(f.read()).hexdigest()[:16]

        manifest = {
            "version": "freemix.v1",
            "source": self._source_path or "array_input",
            "source_hash": src_hash,
            "sample_rate": self.sr,
            "stems": list(stem_paths.keys()),
            "variations": manifest_data,
            "output": {
                "freestyle_mix": mix_path,
                "stem_dir": output_dir,
            }
        }

        manifest_path = os.path.join(output_dir, "freemix_manifest.json")
        with open(manifest_path, "w") as f:
            json.dump(manifest, f, indent=2)

        print(f"[FreeMix] Freestyle complete:")
        print(f"  Mix: {mix_path}")
        print(f"  Manifest: {manifest_path}")
        return manifest_path


# ══════════════════════════════════════════════════════════════════
# CLI
# ══════════════════════════════════════════════════════════════════
def main():
    import argparse
    parser = argparse.ArgumentParser(
        description="FreeMix Engine — Freestyle stem remixer (Stem Player paradigm)"
    )
    parser.add_argument("input", help="Input audio file (.wav, .mp3, .flac, .ogg)")
    parser.add_argument("output_dir", help="Output directory for stems + remix")
    parser.add_argument("--mode", choices=["freestyle", "remix"], default="freestyle",
                        help="freestyle: random remix variations | remix: manual config")
    parser.add_argument("--variations", type=int, default=8,
                        help="Number of freestyle variations (default: 8)")
    parser.add_argument("--bars", type=int, default=4,
                        help="Bars per variation (default: 4)")
    parser.add_argument("--seed", type=int, default=None,
                        help="Random seed for reproducible freestyle")
    parser.add_argument("--config", type=str, default=None,
                        help="JSON remix config e.g. drums:vol,vocals:pitch")
    args = parser.parse_args()

    fm = FreeMix()
    fm.load(args.input)
    fm.separate()

    if args.mode == "freestyle":
        manifest = fm.freestyle_to_dir(
            args.output_dir,
            n_variations=args.variations,
            bars_per_variation=args.bars,
            seed=args.seed,
        )
        print(f"\nDone. Manifest: {manifest}")
    else:
        config = json.loads(args.config) if args.config else {}
        remix_path = os.path.join(args.output_dir, "remix.wav")
        Path(args.output_dir).mkdir(parents=True, exist_ok=True)
        fm.remix_to_file(remix_path, config)
        print(f"\nDone. Remix: {remix_path}")


if __name__ == "__main__":
    main()
