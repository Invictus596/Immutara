"""Unit and integration tests for the Immutara CV worker.

Run with (from the `py/` directory):

    IMMUTARA_CV_MODELS_DIR=../models \
    python -m unittest discover -s tests -v

Requires the models downloaded (`python3 ../models/download_models.py`) and
the `opencv-python` dependency installed.
"""

import hashlib
import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PY_ROOT = Path(__file__).resolve().parents[1]
MODELS_DIR = Path(
    os.environ.get("IMMUTARA_CV_MODELS_DIR", REPO_ROOT / "models")
)
FIXTURES = Path(__file__).resolve().parent / "fixtures"
FACE = FIXTURES / "face_lena.jpg"
NO_FACE = FIXTURES / "no_face_blank.png"

sys.path.insert(0, str(PY_ROOT))

from immutara_cv.analysis import FaceAnalyzer  # noqa: E402
from immutara_cv.errors import (  # noqa: E402
    CvError,
    IMAGE_UNREADABLE,
    MODEL_MISSING,
)
from immutara_cv.face_worker import SCHEMA_VERSION, main as worker_main  # noqa: E402


@unittest.skipUnless(MODELS_DIR.exists(), "models directory not present")
class FaceAnalyzerTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.analyzer = FaceAnalyzer(str(MODELS_DIR), score_threshold=0.9)

    def test_detects_face_and_embeds(self):
        result = self.analyzer.analyze(str(FACE))
        self.assertGreaterEqual(result.face_count, 1)
        self.assertGreater(result.confidence, 0.0)
        self.assertLessEqual(result.confidence, 1.0)
        x, y, w, h = result.bounding_box
        self.assertGreater(w, 0)
        self.assertGreater(h, 0)
        self.assertEqual(result.embedding_dimensions, 128)
        self.assertEqual(len(result.embedding), 128)
        self.assertTrue(result.embedding.any())

    def test_embedding_is_deterministic(self):
        first = self.analyzer.analyze(str(FACE)).embedding
        second = self.analyzer.analyze(str(FACE)).embedding
        self.assertTrue(all(float(a) == float(b) for a, b in zip(first, second)))

    def test_no_face_returns_zero_count(self):
        result = self.analyzer.analyze(str(NO_FACE))
        self.assertEqual(result.face_count, 0)
        self.assertEqual(result.embedding_dimensions, 0)

    def test_missing_image_raises_unreadable(self):
        with self.assertRaises(CvError) as ctx:
            self.analyzer.analyze(str(FIXTURES / "does_not_exist.jpg"))
        self.assertEqual(ctx.exception.code, IMAGE_UNREADABLE)

    def test_primary_face_selection_is_deterministic(self):
        import numpy as np

        # Mirrors the OpenCV 5.x layout: indices 0..3 box, 4..13 landmarks
        # (large coordinates, not in [0,1]), index 14 = score in [0,1].
        lm = list(range(4, 14))
        faces = np.array(
            [
                [0, 0, 100, 100] + lm + [0.95],
                [0, 0, 300, 300] + lm + [0.95],
                [0, 0, 150, 150] + lm + [0.99],
            ],
            dtype=np.float32,
        )
        score_at, _row, box, score = self.analyzer._primary_face(faces)
        # Score lives in the last column for the OpenCV 5.x layout.
        self.assertEqual(score_at, 14)
        self.assertAlmostEqual(score, 0.99)
        x, y, w, h = box
        self.assertEqual((w, h), (150, 150))

    def test_missing_models_raises_model_missing(self):
        with self.assertRaises(CvError) as ctx:
            FaceAnalyzer(str(PY_ROOT / "not_a_models_dir"))
        self.assertEqual(ctx.exception.code, MODEL_MISSING)


@unittest.skipUnless(MODELS_DIR.exists(), "models directory not present")
class WorkerProtocolTest(unittest.TestCase):
    """Exercises the worker over its real NDJSON stdin/stdout transport."""

    @classmethod
    def setUpClass(cls):
        env = dict(os.environ)
        env["IMMUTARA_CV_MODELS_DIR"] = str(MODELS_DIR)
        env["PYTHONPATH"] = str(PY_ROOT)
        cls.env = env

    def _run(self, payloads):
        lines = [json.dumps(p) for p in payloads]
        data = ("\n".join(lines) + "\n").encode()
        proc = subprocess.run(
            [sys.executable, "-m", "immutara_cv.face_worker"],
            input=data,
            capture_output=True,
            env=self.env,
            cwd=str(PY_ROOT),
        )
        responses = [
            json.loads(line) for line in proc.stdout.decode().splitlines() if line.strip()
        ]
        return proc.returncode, responses

    def test_ok_response_shape(self):
        code, resp = self._run([{"schema_version": 1, "image_path": str(FACE)}])
        self.assertEqual(code, 0)
        self.assertEqual(len(resp), 1)
        r = resp[0]
        self.assertEqual(r["schema_version"], SCHEMA_VERSION)
        self.assertEqual(r["status"], "ok")
        self.assertEqual(r["model"]["detector"], "YuNet")
        self.assertEqual(r["model"]["recognizer"], "SFace")
        self.assertEqual(r["embedding"]["dimensions"], 128)
        self.assertEqual(len(r["embedding"]["values"]), 128)
        bb = r["selected_face"]["bounding_box"]
        for k in ("x", "y", "width", "height"):
            self.assertIn(k, bb)
        # Embedding values must hash like a fixed-size byte sequence for Rust.
        raw = struct_pack_floats(r["embedding"]["values"])
        self.assertEqual(len(raw), 128 * 4)
        digest = hashlib.sha256(raw).hexdigest()
        self.assertEqual(len(digest), 64)

    def test_no_face_error(self):
        code, resp = self._run([{"schema_version": 1, "image_path": str(NO_FACE)}])
        self.assertEqual(code, 0)
        self.assertEqual(resp[0]["status"], "error")
        self.assertEqual(resp[0]["error"]["code"], "no_face")

    def test_malformed_json(self):
        data = b"this is not json\n"
        proc = subprocess.run(
            [sys.executable, "-m", "immutara_cv.face_worker"],
            input=data,
            capture_output=True,
            env=self.env,
            cwd=str(PY_ROOT),
        )
        resp = json.loads(proc.stdout.decode().strip())
        self.assertEqual(resp["status"], "error")
        self.assertEqual(resp["error"]["code"], "malformed_request")

    def test_unsupported_schema(self):
        code, resp = self._run([{"schema_version": 99, "image_path": str(FACE)}])
        self.assertEqual(resp[0]["status"], "error")
        self.assertEqual(resp[0]["error"]["code"], "unsupported_request")

    def test_image_unreadable(self):
        code, resp = self._run(
            [{"schema_version": 1, "image_path": str(FIXTURES / "nope.png")}]
        )
        self.assertEqual(resp[0]["status"], "error")
        self.assertEqual(resp[0]["error"]["code"], "image_unreadable")

    def test_multiple_requests_sequential(self):
        code, resp = self._run(
            [
                {"schema_version": 1, "image_path": str(NO_FACE)},
                {"schema_version": 1, "image_path": str(FACE)},
            ]
        )
        self.assertEqual(code, 0)
        self.assertEqual(len(resp), 2)
        self.assertEqual(resp[0]["error"]["code"], "no_face")
        self.assertEqual(resp[1]["status"], "ok")

    def test_worker_startup_fails_without_models_env(self):
        env = dict(os.environ)
        env.pop("IMMUTARA_CV_MODELS_DIR", None)
        env["PYTHONPATH"] = str(PY_ROOT)
        proc = subprocess.run(
            [sys.executable, "-m", "immutara_cv.face_worker"],
            input=b"",
            capture_output=True,
            env=env,
            cwd=str(PY_ROOT),
        )
        self.assertEqual(proc.returncode, 1)


def struct_pack_floats(values):
    import struct

    return struct.pack("<%df" % len(values), *values)


if __name__ == "__main__":
    unittest.main()
