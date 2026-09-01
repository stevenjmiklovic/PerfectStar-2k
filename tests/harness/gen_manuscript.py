#!/usr/bin/env python3
"""Generate a large, deterministic manuscript fixture for performance testing.

Guards constraint **C6**: `pstar` must stay responsive (sub-frame keystroke
latency) on manuscripts of at least 300,000 words. The fixture is realistic
prose shape — chapters (`#` headings), paragraphs, occasional Markdown emphasis
and `..` note lines — so the incremental word-count and style/spell scanners
that later tasks add are exercised the way a real book would exercise them.

Deterministic (fixed PRNG seed) so a regression is reproducible and the file
only needs regenerating when the target size changes. The output is large
(~1.8 MB) and therefore git-ignored; regenerate on demand:

    python3 tests/harness/gen_manuscript.py            # default 300k words
    python3 tests/harness/gen_manuscript.py --words 50000 --out /tmp/small.md
"""

import argparse
import random
from pathlib import Path

# A small, fixed vocabulary. Deterministic output matters more than variety;
# the counters and scanners under test don't care about the dictionary.
WORDS = (
    "the quick brown fox jumps over a lazy dog while morning light spilled "
    "across the valley and the river carried its slow secrets toward the sea "
    "she remembered nothing of that winter only the sound of wind against "
    "glass and the way he had looked away when she asked the question that "
    "mattered most between them now stretched a silence wide as any ocean"
).split()

# Repository root is two levels up from tests/harness/.
REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUT = REPO_ROOT / "tests" / "fixtures" / "manuscript-300k.md"


def sentence(rng):
    n = rng.randint(6, 18)
    words = [rng.choice(WORDS) for _ in range(n)]
    # Occasionally emphasize a word so the Markdown scanner has real work.
    if rng.random() < 0.05:
        i = rng.randrange(len(words))
        words[i] = f"*{words[i]}*"
    words[0] = words[0].capitalize()
    return " ".join(words) + rng.choice([".", ".", ".", "?", "!"])


def generate(target_words, out_path):
    rng = random.Random(0x5EED)  # fixed seed → identical fixture every run
    lines = []
    words_written = 0
    chapter = 0

    while words_written < target_words:
        chapter += 1
        lines.append(f"# Chapter {chapter}")
        lines.append("")
        # ~8-15 paragraphs per chapter.
        for _ in range(rng.randint(8, 15)):
            para = " ".join(sentence(rng) for _ in range(rng.randint(3, 7)))
            lines.append(para)
            lines.append("")
            words_written += len(para.split())
            # A sprinkling of note-to-self lines (stripped from prose counts).
            if rng.random() < 0.04:
                lines.append(f".. revisit this beat in chapter {chapter}")
                lines.append("")

    out_path.parent.mkdir(parents=True, exist_ok=True)
    text = "\n".join(lines)
    out_path.write_text(text, encoding="utf-8")
    return words_written, len(text)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--words", type=int, default=300_000,
                    help="approximate word count (default: 300000)")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT,
                    help=f"output path (default: {DEFAULT_OUT})")
    args = ap.parse_args()

    words, nbytes = generate(args.words, args.out)
    print(f"Wrote {args.out} — {words:,} words, {nbytes:,} bytes")


if __name__ == "__main__":
    main()
