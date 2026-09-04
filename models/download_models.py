#!/usr/bin/env python3
"""Download the Immutara face-analysis ONNX models from official sources.

This script fetches the OpenCV Zoo YuNet (detector) and SFace (recognizer)
models into the `models/` directory. Run once during setup; models are not
silently downloaded during pipeline execution.

Usage:
    python3 models/download_models.py [--models-dir DIR]

Sources & licenses:
  - YuNet face detector (CommitPackage "2023mar"), OpenCV Zoo
      URL: https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx
      License: Apache-2.0
  - SFace face recognizer ("2021dec"), OpenCV Zoo
      URL (LFS media): https://media.githubusercontent.com/media/opencv/opencv_zoo/main/models/face_recognition_sface/face_recognition_sface_2021dec.onnx
      License: Apache-2.0

Attribution:
  - YuNet: Wei Wu, Hong Peng, Shuo Yu
  - SFace: Yujia Zhang, et al. (SFace: Sigmoid-Constrained Hypersphere Loss for
    Robust Face Recognition, arXiv:2205.12010) — model distributed via OpenCV
    Zoo under the OpenCV license set for the zoo models.
"""

import argparse
import platform
import shutil
import sys
import urllib.request
from pathlib import Path

MODELS = {
    "face_detection_yunet": {
        "file": "face_detection_yunet_2023mar.onnx",
        "url": (
            "https://github.com/opencv/opencv_zoo/raw/main/"
            "models/face_detection_yunet/face_detection_yunet_2023mar.onnx"
        ),
    },
    "face_recognition_sface": {
        "file": "face_recognition_sface_2021dec.onnx",
        "url": (
            "https://media.githubusercontent.com/media/opencv/opencv_zoo/main/"
            "models/face_recognition_sface/face_recognition_sface_2021dec.onnx"
        ),
    },
}


def _block_hook():
    import time

    def hook(blocks, block_size, total_size):
        if total_size <= 0:
            return
        done = blocks * block_size
        pct = min(100.0, done * 100.0 / total_size)
        sys.stderr.write(f"\r  {pct:5.1f}%  {done/total_size*100:6.1f}")
        sys.stderr.flush()

    return hook


def download(url: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    sys.stderr.write(f"Downloading {url.split('/')[-1]}\n")
    tmp = dest.with_suffix(dest.suffix + ".part")
    req = urllib.request.Request(url, headers={"User-Agent": "immutara-models/0.1"})
    with urllib.request.urlopen(req, timeout=60) as resp, open(tmp, "wb") as out:
        shutil.copyfileobj(resp, out)
    tmp.replace(dest)
    sys.stderr.write(f" -> {dest} ({dest.stat().st_size} bytes)\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--models-dir",
        default=str(Path(__file__).resolve().parent),
        help="Root models directory (default: alongside this script).",
    )
    args = parser.parse_args()
    root = Path(args.models_dir).resolve()

    for sub, spec in MODELS.items():
        dest = root / sub / spec["file"]
        if dest.exists() and dest.stat().st_size > 0:
            print(f"OK   already present: {dest}")
            continue
        try:
            download(spec["url"], dest)
        except Exception as exc:  # noqa: BLE001 - report and continue
            print(f"FAIL {sub}: {exc}", file=sys.stderr)
            return 1

    print("All models downloaded.")
    print("Note: non-x86/Apple-silicon hosts may need `opencv-python` wheels")
    print(f"built for {platform.system()} {platform.machine()}.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
