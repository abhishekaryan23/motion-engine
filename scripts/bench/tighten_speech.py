#!/usr/bin/env python3
"""Sprint 0.23 A4-short: make the continuous-take speech fixtures.

The variety bench's `say` fixtures leave 1.2-1.3 s between sentences in places;
a real narration (the voice rule: one take, continuous script) leaves
0.25-0.6 s. This tool re-times a speech map so that EVERY gap between two
consecutive sentences is exactly GAP seconds (default 0.30): the words of each
sentence move together (each keeps its own duration), the sentences keep their
length, the first sentence starts where it did, and the map's `duration` is
shortened by what was removed (the tail after the last word is kept).

Deterministic: no clock, no randomness, same input -> same bytes.

Usage:
    tighten_speech.py IN.speech.json OUT.speech.json [--gap 0.30]
    tighten_speech.py --all [--gap 0.30]     # the eight bench fixtures ->
                                             # golden/fixtures/variety/tight/

A word belongs to the sentence whose [start, end] holds its start time (a word
before the first sentence stays; a word between two sentences joins the earlier
one). Nothing else in the map changes.
"""
import argparse
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..', '..', '..'))
FIXTURES = os.path.join(ROOT, 'golden', 'fixtures', 'variety')
EPS = 1e-9


def r6(x):
    return round(x, 6)


def tighten(speech, gap):
    """A copy of `speech` with every inter-sentence gap set to `gap` seconds."""
    out = json.loads(json.dumps(speech))
    sentences = out.get('sentences', [])
    words = out.get('words', [])
    if not sentences:
        return out
    # Which sentence each word is spoken in (by its start time).
    owner = []
    for w in words:
        k = 0
        for i, s in enumerate(sentences):
            if w['start'] >= s['start'] - EPS:
                k = i
        owner.append(k)
    # How far each sentence moves back: the gap it had minus the gap it gets.
    shift = [0.0] * len(sentences)
    removed = 0.0
    for i in range(1, len(sentences)):
        had = sentences[i]['start'] - sentences[i - 1]['end']
        removed += had - gap
        shift[i] = removed
    for w, k in zip(words, owner):
        w['start'] = r6(w['start'] - shift[k])
        w['end'] = r6(w['end'] - shift[k])
    for s, d in zip(sentences, shift):
        s['start'] = r6(s['start'] - d)
        s['end'] = r6(s['end'] - d)
    out['duration'] = r6(speech['duration'] - removed)
    return out


def dump(path, speech):
    with open(path, 'w') as f:
        json.dump(speech, f, indent=2)
        f.write('\n')


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('paths', nargs='*', help='IN.speech.json OUT.speech.json')
    ap.add_argument('--gap', type=float, default=0.30,
                    help='seconds between consecutive sentences (default 0.30)')
    ap.add_argument('--all', action='store_true',
                    help='tighten every golden/fixtures/variety/*.speech.json into tight/')
    args = ap.parse_args()
    if args.all:
        out_dir = os.path.join(FIXTURES, 'tight')
        os.makedirs(out_dir, exist_ok=True)
        for name in sorted(os.listdir(FIXTURES)):
            if not name.endswith('.speech.json'):
                continue
            with open(os.path.join(FIXTURES, name)) as f:
                speech = json.load(f)
            dump(os.path.join(out_dir, name), tighten(speech, args.gap))
            print(f'tight/{name}')
        return 0
    if len(args.paths) != 2:
        ap.error('give IN.speech.json OUT.speech.json, or --all')
    with open(args.paths[0]) as f:
        speech = json.load(f)
    dump(args.paths[1], tighten(speech, args.gap))
    return 0


if __name__ == '__main__':
    sys.dont_write_bytecode = True
    sys.exit(main())
