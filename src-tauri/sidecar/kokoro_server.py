"""CPrep text-to-speech sidecar.

Keeps Kokoro-82M loaded and answers one JSON request per line on stdin:
    {"id": 1, "text": "Hello", "voice": "af_heart", "speed": 1.0}
with one JSON line on stdout:
    {"id": 1, "wav_b64": "...", "seconds": 0.8, "ms": 410}   or   {"id": 1, "error": "..."}
The first line it prints is {"ready": true} once the model is loaded.
"""

import base64
import io
import json
import os
import sys
import time
import warnings

warnings.filterwarnings("ignore")
os.environ.setdefault("HF_HUB_DISABLE_TELEMETRY", "1")

# Library chatter (spaCy, torch) must not corrupt the JSON protocol on stdout.
_real_stdout = sys.stdout
sys.stdout = sys.stderr

import numpy as np  # noqa: E402
import soundfile as sf  # noqa: E402
from kokoro import KPipeline  # noqa: E402

SAMPLE_RATE = 24000
_pipelines = {}


def pipeline_for(voice: str) -> KPipeline:
    # Voice prefix picks the language: a* = American English, b* = British English.
    lang = "b" if voice.startswith("b") else "a"
    if lang not in _pipelines:
        _pipelines[lang] = KPipeline(lang_code=lang, repo_id="hexgrad/Kokoro-82M")
    return _pipelines[lang]


def reply(obj) -> None:
    _real_stdout.write(json.dumps(obj) + "\n")
    _real_stdout.flush()


def synth(text: str, voice: str, speed: float) -> bytes:
    chunks = [audio for _, _, audio in pipeline_for(voice)(text, voice=voice, speed=speed)]
    if not chunks:
        raise ValueError("no audio generated")
    audio = np.concatenate([np.asarray(c, dtype=np.float32) for c in chunks])
    buf = io.BytesIO()
    sf.write(buf, audio, SAMPLE_RATE, format="WAV", subtype="PCM_16")
    return buf.getvalue()


def main() -> None:
    pipeline_for("af_heart")
    synth("Ready.", "af_heart", 1.0)  # warm up so the first real sentence is fast
    reply({"ready": True})
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        req_id = None
        try:
            req = json.loads(line)
            req_id = req.get("id")
            text = str(req.get("text", "")).strip()[:2000]
            if not text:
                raise ValueError("empty text")
            t = time.time()
            wav = synth(text, str(req.get("voice") or "af_heart"), float(req.get("speed") or 1.0))
            reply({
                "id": req_id,
                "wav_b64": base64.b64encode(wav).decode("ascii"),
                "seconds": round((len(wav) - 44) / 2 / SAMPLE_RATE, 2),
                "ms": int((time.time() - t) * 1000),
            })
        except Exception as e:  # keep serving after a bad request
            reply({"id": req_id, "error": f"{type(e).__name__}: {e}"})


if __name__ == "__main__":
    main()
