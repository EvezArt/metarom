#!/usr/bin/env python3
"""
MetaROM Network Crystallizer
One system. Many ROMs. One mind.

Turns a pool of Game Boy ROMs into a single unified training crystal
that borrows patterns from every game it runs and crystallizes
cross-ROM behavior into EVEZ-OS console_war_trainer epochs.

Usage:
  python network_crystallizer.py roms/ output/ --frames 60 --epoch gen1_nes
  python network_crystallizer.py roms/ output/ --frames 120 --crystallize --broadcast

Architecture:
  ROM pool → headless runner → replay capture → feature extraction
       → cross-ROM crystal → training manifest (mrom.crystal.v1)
       → EVEZ-OS epoch pipeline
"""

import os, json, hashlib, math, random, time, struct
from pathlib import Path
from typing import Dict, List, Optional, Tuple, Any
from dataclasses import dataclass, field, asdict
import numpy as np

# ══════════════════════════════════════════════════════════════════
# MROM BINARY RUNNER
# Runs the compiled letsplay_train binary (Rust gb-core) for each ROM
# Captures training JSON output → feeds into crystallizer
# If binary not found: falls back to pure-Python ROM header parser
# ══════════════════════════════════════════════════════════════════

LETSPLAY_BINARY = os.environ.get("METAROM_BIN", "./target/release/letsplay_train")

@dataclass
class RomHeader:
    path: str
    title: str
    sha256: str
    cart_type: int
    rom_size_kb: int
    ram_size_kb: int
    is_cgb: bool
    is_sgb: bool
    licensee: int

    @classmethod
    def parse(cls, path: str) -> "RomHeader":
        with open(path, "rb") as f:
            data = f.read()
        sha256 = hashlib.sha256(data).hexdigest()
        title_bytes = data[0x134:0x143]
        title = title_bytes.split(b"\x00")[0].decode("ascii", errors="replace").strip()
        cart_type  = data[0x147] if len(data) > 0x147 else 0
        rom_code   = data[0x148] if len(data) > 0x148 else 0
        ram_code   = data[0x149] if len(data) > 0x149 else 0
        cgb_flag   = data[0x143] if len(data) > 0x143 else 0
        sgb_flag   = data[0x146] if len(data) > 0x146 else 0
        licensee   = data[0x14B] if len(data) > 0x14B else 0

        rom_kb_map = {0:32, 1:64, 2:128, 3:256, 4:512, 5:1024, 6:2048, 7:4096, 8:8192}
        ram_kb_map = {0:0, 1:2, 2:8, 3:32, 4:128, 5:64}
        rom_kb = rom_kb_map.get(rom_code, 32)
        ram_kb = ram_kb_map.get(ram_code, 0)

        return cls(
            path=path, title=title, sha256=sha256,
            cart_type=cart_type, rom_size_kb=rom_kb, ram_size_kb=ram_kb,
            is_cgb=cgb_flag in (0x80, 0xC0), is_sgb=sgb_flag == 0x03,
            licensee=licensee
        )


# ══════════════════════════════════════════════════════════════════
# FEATURE EXTRACTOR
# Turns a mrom.train.json record into a feature vector
# Features: PC distribution, frame entropy, framebuffer statistics
# ══════════════════════════════════════════════════════════════════

@dataclass
class RomFeatures:
    rom_title: str
    rom_sha256: str
    epoch: str
    is_cgb: bool
    frame_count: int
    # PC histogram (256 buckets for PC high-byte)
    pc_histogram: List[float] = field(default_factory=lambda: [0.0]*256)
    # Frame entropy (how much the framebuffer changes each frame)
    frame_entropy: List[float] = field(default_factory=list)
    # Framebuffer statistics
    fb_mean: float = 0.0
    fb_std: float = 0.0
    fb_activity: float = 0.0   # fraction of pixels that changed vs previous frame
    # Behavior signature: compact 64-dim vector
    behavior_vec: List[float] = field(default_factory=lambda: [0.0]*64)


class FeatureExtractor:
    def extract_from_training_json(self, json_path: str) -> Optional[RomFeatures]:
        """Extract feature vector from a mrom.train.v1 JSON file."""
        try:
            with open(json_path) as f:
                data = json.load(f)
        except Exception as e:
            print(f"  [warn] Cannot read {json_path}: {e}")
            return None

        records = data.get("frames", [])
        if not records:
            return None

        rom_info = data.get("rom", {})
        header = data.get("header", {})
        epoch = data.get("epoch", "gen1_nes")

        feat = RomFeatures(
            rom_title  = rom_info.get("title", "UNKNOWN"),
            rom_sha256 = rom_info.get("sha256", ""),
            epoch      = epoch,
            is_cgb     = header.get("is_cgb", False),
            frame_count= len(records),
        )

        pc_hist = [0] * 256
        fb_vals = []
        prev_fb = None

        for rec in records:
            # PC histogram
            pc = rec.get("cpu", {}).get("pc", 0)
            pc_hist[pc >> 8] += 1

            # Framebuffer stats
            fb_hex = rec.get("fb", "")
            if fb_hex and len(fb_hex) >= 2:
                fb_bytes = bytes.fromhex(fb_hex[:9216]) if len(fb_hex) >= 9216 else None
                if fb_bytes:
                    vals = list(fb_bytes)
                    fb_vals.extend(vals[:16])  # sample first 16 pixels per frame
                    if prev_fb:
                        changed = sum(1 for a,b in zip(vals, prev_fb) if a != b)
                        feat.frame_entropy.append(changed / len(vals))
                    prev_fb = vals

        # Normalize PC histogram
        total = sum(pc_hist) or 1
        feat.pc_histogram = [v/total for v in pc_hist]

        # FB stats
        if fb_vals:
            arr = np.array(fb_vals, dtype=float) / 255.0
            feat.fb_mean = float(np.mean(arr))
            feat.fb_std  = float(np.std(arr))

        feat.fb_activity = float(np.mean(feat.frame_entropy)) if feat.frame_entropy else 0.0

        # Compute 64-dim behavior vector:
        # [PC_hist_coarse[32], fb_mean, fb_std, fb_activity, epoch_encoding[16], zeros[14]]
        pc_coarse = [sum(feat.pc_histogram[i*8:(i+1)*8]) for i in range(32)]
        epoch_vec = _epoch_to_vec(epoch)
        feat.behavior_vec = pc_coarse + [feat.fb_mean, feat.fb_std, feat.fb_activity] + epoch_vec + [0.0]*13
        feat.behavior_vec = feat.behavior_vec[:64]

        return feat

    def extract_from_header(self, header: RomHeader, n_frames: int = 60) -> RomFeatures:
        """Synthetic features when no training JSON available — from ROM header only."""
        epoch = "gen2_snes_genesis" if header.is_cgb else "gen1_nes"
        feat = RomFeatures(
            rom_title=header.title, rom_sha256=header.sha256,
            epoch=epoch, is_cgb=header.is_cgb, frame_count=n_frames,
        )
        # Synthetic PC histogram biased by cart type
        rng = random.Random(int(header.sha256[:8], 16))
        pc_hist = [rng.random() for _ in range(256)]
        total = sum(pc_hist)
        feat.pc_histogram = [v/total for v in pc_hist]
        epoch_vec = _epoch_to_vec(epoch)
        feat.behavior_vec = [sum(feat.pc_histogram[i*8:(i+1)*8]) for i in range(32)]
        feat.behavior_vec += [0.5, 0.2, 0.3] + epoch_vec + [0.0]*13
        feat.behavior_vec = feat.behavior_vec[:64]
        return feat


def _epoch_to_vec(epoch: str) -> List[float]:
    """One-hot-ish encoding for 9 console generations."""
    epochs = ["gen1_nes", "gen2_snes_genesis", "gen3_n64_ps1", "gen4_ps2_gc",
              "gen5_wii_360", "gen6_ps4_xone", "gen7_switch", "gen8_ps5", "gen9_cloud"]
    vec = [0.0] * 16
    if epoch in epochs:
        vec[epochs.index(epoch)] = 1.0
    return vec


# ══════════════════════════════════════════════════════════════════
# CROSS-ROM CRYSTAL
# The core "borrows and trains from many games" engine
# Computes pairwise cosine similarity, detects behavior clusters,
# generates consensus patterns, borrows best patterns across ROMs
# ══════════════════════════════════════════════════════════════════

@dataclass
class BehaviorCluster:
    cluster_id: int
    rom_titles: List[str]
    centroid: List[float]
    cohesion: float  # average pairwise similarity within cluster
    epoch: str
    dominant_epoch: str

@dataclass 
class CrystalManifest:
    version: str = "mrom.crystal.v1"
    rom_count: int = 0
    frame_total: int = 0
    epochs_seen: List[str] = field(default_factory=list)
    clusters: List[BehaviorCluster] = field(default_factory=list)
    cross_rom_patterns: List[Dict] = field(default_factory=list)
    consensus_behavior_vec: List[float] = field(default_factory=lambda: [0.0]*64)
    crystallization_score: float = 0.0  # 0-1, higher = more cross-ROM pattern sharing
    roms: List[Dict] = field(default_factory=list)


class CrossRomCrystal:
    def __init__(self, min_similarity: float = 0.7):
        self.min_similarity = min_similarity
        self.features: List[RomFeatures] = []

    def ingest(self, feat: RomFeatures):
        self.features.append(feat)

    def _cosine_sim(self, a: List[float], b: List[float]) -> float:
        an = np.array(a); bn = np.array(b)
        denom = np.linalg.norm(an) * np.linalg.norm(bn)
        if denom < 1e-9: return 0.0
        return float(np.dot(an, bn) / denom)

    def borrow(self) -> CrystalManifest:
        """
        Crystallize: find shared patterns across all ingested ROMs.
        Returns CrystalManifest with clusters and consensus vector.
        """
        n = len(self.features)
        if n == 0:
            return CrystalManifest()

        # Build similarity matrix
        sim_matrix = np.zeros((n, n))
        for i in range(n):
            for j in range(n):
                sim_matrix[i][j] = self._cosine_sim(
                    self.features[i].behavior_vec,
                    self.features[j].behavior_vec
                )

        # Simple greedy clustering
        assigned = [-1] * n
        clusters: List[List[int]] = []
        for i in range(n):
            if assigned[i] != -1: continue
            cluster = [i]
            assigned[i] = len(clusters)
            for j in range(i+1, n):
                if assigned[j] == -1 and sim_matrix[i][j] >= self.min_similarity:
                    cluster.append(j)
                    assigned[j] = len(clusters)
            clusters.append(cluster)

        # Build cluster objects
        behavior_clusters = []
        for cid, members in enumerate(clusters):
            vecs = [self.features[m].behavior_vec for m in members]
            centroid = list(np.mean(np.array(vecs), axis=0))

            # Cohesion = avg pairwise similarity
            if len(members) > 1:
                pairs = [(i,j) for i in members for j in members if i < j]
                cohesion = np.mean([sim_matrix[i][j] for i,j in pairs])
            else:
                cohesion = 1.0

            epochs = [self.features[m].epoch for m in members]
            from collections import Counter
            dominant = Counter(epochs).most_common(1)[0][0]

            behavior_clusters.append(BehaviorCluster(
                cluster_id=cid,
                rom_titles=[self.features[m].rom_title for m in members],
                centroid=centroid,
                cohesion=float(cohesion),
                epoch=dominant,
                dominant_epoch=dominant
            ))

        # Consensus behavior vector = mean of all feature vectors
        all_vecs = np.array([f.behavior_vec for f in self.features])
        consensus = list(np.mean(all_vecs, axis=0))

        # Cross-ROM patterns: pairs with high similarity across different epochs
        cross_patterns = []
        for i in range(n):
            for j in range(i+1, n):
                if (sim_matrix[i][j] >= self.min_similarity and 
                    self.features[i].epoch != self.features[j].epoch):
                    cross_patterns.append({
                        "rom_a": self.features[i].rom_title,
                        "rom_b": self.features[j].rom_title,
                        "epoch_a": self.features[i].epoch,
                        "epoch_b": self.features[j].epoch,
                        "similarity": round(float(sim_matrix[i][j]), 4),
                        "pattern": "cross_epoch_behavior_match"
                    })

        # Crystallization score: fraction of ROMs in multi-ROM clusters
        in_shared = sum(1 for c in clusters if len(c) > 1)
        crystal_score = in_shared / n if n > 0 else 0.0

        # Build epoch set
        epochs_seen = sorted(set(f.epoch for f in self.features))

        manifest = CrystalManifest(
            rom_count=n,
            frame_total=sum(f.frame_count for f in self.features),
            epochs_seen=epochs_seen,
            clusters=behavior_clusters,
            cross_rom_patterns=cross_patterns[:50],  # cap
            consensus_behavior_vec=consensus,
            crystallization_score=round(crystal_score, 4),
            roms=[{
                "title": f.rom_title,
                "sha256": f.rom_sha256[:16],
                "epoch": f.epoch,
                "is_cgb": f.is_cgb,
                "frame_count": f.frame_count,
                "fb_activity": round(f.fb_activity, 4),
            } for f in self.features]
        )
        return manifest

    def save_manifest(self, manifest: CrystalManifest, path: str):
        """Serialize crystal manifest to JSON."""
        def _serial(obj):
            if isinstance(obj, (BehaviorCluster,)):
                return {k: _serial(v) for k,v in asdict(obj).items()}
            if isinstance(obj, list):
                return [_serial(x) for x in obj]
            if isinstance(obj, float):
                return round(obj, 6)
            return obj

        data = {
            "version": manifest.version,
            "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "rom_count": manifest.rom_count,
            "frame_total": manifest.frame_total,
            "epochs_seen": manifest.epochs_seen,
            "crystallization_score": manifest.crystallization_score,
            "consensus_behavior_vec": [round(v,6) for v in manifest.consensus_behavior_vec],
            "clusters": [
                {
                    "cluster_id": c.cluster_id,
                    "rom_titles": c.rom_titles,
                    "cohesion": round(c.cohesion, 4),
                    "dominant_epoch": c.dominant_epoch,
                    "centroid_norm": round(float(np.linalg.norm(c.centroid)), 4),
                }
                for c in manifest.clusters
            ],
            "cross_rom_patterns": manifest.cross_rom_patterns,
            "roms": manifest.roms,
        }
        with open(path, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
        print(f"[Crystal] Manifest saved: {path}")


# ══════════════════════════════════════════════════════════════════
# LIVE BROADCAST SERVER (no external deps)
# Streams mrom.snap.v1 JSON frames over stdout for piping to
# EVEZ-OS dashboard or a WebSocket bridge
# ══════════════════════════════════════════════════════════════════

class LiveBroadcast:
    """
    Minimal broadcast: writes newline-delimited JSON to stdout.
    Pipe to websocat or a Node.js WebSocket bridge for live dashboard.

    Format: one JSON object per line (NDJSON)
    {"type":"frame","rom":"POKEMON_RED","data": <mrom.snap.v1>}
    {"type":"crystal_update","data": <partial crystal manifest>}
    {"type":"epoch_advance","from":"gen1_nes","to":"gen2_snes_genesis"}
    """
    def __init__(self, enabled: bool = False):
        self.enabled = enabled
        self.frame_count = 0

    def emit_frame(self, rom_title: str, snap_json: str):
        if not self.enabled: return
        print(json.dumps({"type": "frame", "rom": rom_title, "t": self.frame_count, "data": snap_json}), flush=True)
        self.frame_count += 1

    def emit_crystal_update(self, manifest: CrystalManifest):
        if not self.enabled: return
        print(json.dumps({
            "type": "crystal_update",
            "rom_count": manifest.rom_count,
            "crystallization_score": manifest.crystallization_score,
            "epochs_seen": manifest.epochs_seen,
            "cluster_count": len(manifest.clusters),
        }), flush=True)

    def emit_epoch_advance(self, from_epoch: str, to_epoch: str, reason: str):
        if not self.enabled: return
        print(json.dumps({"type": "epoch_advance", "from": from_epoch, "to": to_epoch, "reason": reason}), flush=True)


# ══════════════════════════════════════════════════════════════════
# MAIN PIPELINE
# ══════════════════════════════════════════════════════════════════

class NetworkCrystallizer:
    """
    One system. Many ROMs. One mind.

    for each ROM in pool:
        run letsplay_train OR parse header → features
        ingest into CrossRomCrystal

    crystal.borrow() → CrystalManifest → mrom.crystal.v1
    """

    def __init__(self, output_dir: str, frames_per_rom: int = 60, broadcast: bool = False):
        self.output_dir = Path(output_dir)
        self.output_dir.mkdir(parents=True, exist_ok=True)
        self.frames_per_rom = frames_per_rom
        self.crystal = CrossRomCrystal(min_similarity=0.65)
        self.extractor = FeatureExtractor()
        self.broadcast = LiveBroadcast(enabled=broadcast)
        self.rom_results: List[Dict] = []

    def _run_rom(self, rom_path: str) -> Optional[str]:
        """Run letsplay_train binary → returns path to output JSON, or None."""
        import subprocess
        out_path = self.output_dir / (Path(rom_path).stem + ".mrom.train.json")
        try:
            result = subprocess.run(
                [LETSPLAY_BINARY, str(self.frames_per_rom), str(out_path), rom_path],
                capture_output=True, text=True, timeout=30
            )
            if result.returncode == 0 and out_path.exists():
                return str(out_path)
        except (FileNotFoundError, subprocess.TimeoutExpired):
            pass
        return None

    def run(self, rom_dir: str, epoch_override: Optional[str] = None) -> str:
        """
        Main entry: process all ROMs in rom_dir.
        Returns path to crystal manifest.
        """
        rom_files = sorted(
            [str(p) for p in Path(rom_dir).glob("**/*") if p.suffix.lower() in (".gb", ".gbc", ".rom", ".bin")]
        )
        if not rom_files:
            # If no real ROMs, run in demo mode with synthetic headers
            print("[Crystal] No ROMs found — running in synthetic demo mode")
            rom_files = self._generate_synthetic_roms()

        print(f"[Crystal] Processing {len(rom_files)} ROMs → {self.output_dir}")
        print(f"[Crystal] Frames per ROM: {self.frames_per_rom}")

        for i, rom_path in enumerate(rom_files):
            print(f"\n[Crystal] ROM {i+1}/{len(rom_files)}: {Path(rom_path).name}")
            header = RomHeader.parse(rom_path)
            print(f"  Title: {header.title}, CGB: {header.is_cgb}, Cart: 0x{header.cart_type:02X}, ROM: {header.rom_size_kb}KB")

            # Try real training run first
            train_json = self._run_rom(rom_path)
            if train_json:
                feat = self.extractor.extract_from_training_json(train_json)
                print(f"  Training JSON: {train_json}")
            else:
                feat = self.extractor.extract_from_header(header, self.frames_per_rom)
                print(f"  Header-only features (binary not found)")

            if feat:
                if epoch_override:
                    feat.epoch = epoch_override
                self.crystal.ingest(feat)
                self.rom_results.append({
                    "rom": header.title,
                    "path": rom_path,
                    "epoch": feat.epoch,
                    "is_cgb": header.is_cgb,
                })

            # Live broadcast: emit synthetic frames
            for _ in range(min(3, self.frames_per_rom)):
                snap = json.dumps({"v": "mrom.snap.v1", "rom": header.title, "f": i*60})
                self.broadcast.emit_frame(header.title, snap)

        print(f"\n[Crystal] Crystallizing {len(self.crystal.features)} ROM feature sets...")
        manifest = self.crystal.borrow()

        print(f"[Crystal] Results:")
        print(f"  ROMs: {manifest.rom_count}")
        print(f"  Frames: {manifest.frame_total}")
        print(f"  Epochs: {manifest.epochs_seen}")
        print(f"  Clusters: {len(manifest.clusters)}")
        print(f"  Cross-epoch patterns: {len(manifest.cross_rom_patterns)}")
        print(f"  Crystallization score: {manifest.crystallization_score:.4f}")

        # Save manifest
        manifest_path = str(self.output_dir / "mrom.crystal.json")
        self.crystal.save_manifest(manifest, manifest_path)

        # Broadcast crystal update
        self.broadcast.emit_crystal_update(manifest)

        # Epoch advance detection
        if len(manifest.epochs_seen) >= 2:
            self.broadcast.emit_epoch_advance(manifest.epochs_seen[0], manifest.epochs_seen[-1],
                                               f"Crystal covers {len(manifest.epochs_seen)} epochs")

        return manifest_path

    def _generate_synthetic_roms(self) -> List[str]:
        """Generate minimal synthetic ROM files for demo/testing."""
        import tempfile
        roms = []
        titles = [
            ("TETRIS___", False, 0x00),
            ("METROID2", False, 0x19),  # MBC5
            ("POKEMON_RED", False, 0x13),  # MBC3
            ("ZELDA_DX", True, 0x1B),   # CGB MBC5
            ("MARIO_COLOR", True, 0x19),  # CGB
            ("WARIOLAND2", True, 0x1B),
            ("KIRBY_COLOR", True, 0x19),
            ("DONKEYKONG", False, 0x01),  # MBC1
        ]
        tmpdir = Path(tempfile.mkdtemp(prefix="metarom_synthetic_"))
        for title, cgb, cart in titles:
            rom = bytearray(32768)
            # Header at 0x100
            rom[0x134:0x134+len(title)] = title.encode("ascii")
            rom[0x143] = 0x80 if cgb else 0x00
            rom[0x147] = cart
            rom[0x148] = 0x00  # 32KB
            rom[0x146] = 0x03 if cgb else 0x00
            # Simple checksum
            chk = sum(rom[0x134:0x14D]) & 0xFF
            rom[0x14D] = (0 - chk - 1) & 0xFF
            path = tmpdir / f"{title.strip('_')}.gb"
            with open(path, "wb") as f: f.write(rom)
            roms.append(str(path))
            print(f"  [Synth] Created {path.name}")
        return roms


# ══════════════════════════════════════════════════════════════════
# CLI
# ══════════════════════════════════════════════════════════════════
def main():
    import argparse
    parser = argparse.ArgumentParser(description="MetaROM Network Crystallizer — many ROMs, one mind")
    parser.add_argument("rom_dir", help="Directory containing .gb/.gbc ROM files")
    parser.add_argument("output_dir", help="Output directory for crystal manifest + training JSON")
    parser.add_argument("--frames", type=int, default=60, help="Frames per ROM (default: 60)")
    parser.add_argument("--epoch", default=None, help="Override epoch tag for all ROMs")
    parser.add_argument("--broadcast", action="store_true", help="Emit live NDJSON frames to stdout")
    parser.add_argument("--crystallize", action="store_true", help="Enable cross-ROM crystallization (always on)")
    args = parser.parse_args()

    crystallizer = NetworkCrystallizer(
        output_dir=args.output_dir,
        frames_per_rom=args.frames,
        broadcast=args.broadcast,
    )
    manifest_path = crystallizer.run(args.rom_dir, epoch_override=args.epoch)
    print(f"\nDone. Crystal manifest: {manifest_path}")


if __name__ == "__main__":
    main()
