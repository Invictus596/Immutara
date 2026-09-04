# Test fixtures provenance

These fixtures are committed binary images used to exercise the CV worker
without requiring network access or user-provided photos.

- `face_lena.jpg` - A 224x224 face-focused crop of the classic `lena.jpg`
  test image, distributed with the OpenCV project (`opencv/opencv` samples,
  `samples/data/lena.jpg`, Apache-2.0). The crop centers on the face region
  so YuNet reliably detects a single face at the default 0.9 threshold.
  Redistributed for testing only; see the OpenCV `COPYRIGHT` for attribution.

- `no_face_blank.png` - A 128x128 solid-gray image with no faces. Used to
  verify the worker returns a `no_face` result rather than crashing.

These fixtures are for automated tests and documentation only; they carry no
biometric or personal data beyond third-party test imagery.
