#!/usr/bin/env python3
"""Structural variety metrics over compiled MotionScene files (0.23 review baseline).

Usage:
  metrics.py DIR sum NAME...           per-beat summary of DIR/NAME.motion.json
  metrics.py DIR cmp BASE NAME...      BASE vs each NAME: look/palette/fonts + per-beat
                                       (layer-role Jaccard distance, op-mix distance,
                                        same camera ops, duration difference)

Layer roles are layer ids with the beat prefix removed and digits replaced by '#',
so two beats built by the same template with different text have distance 0.0.
"""
import collections
import json
import re
import sys


def walk(layers, out):
    for l in layers:
        out.append(l)
        if l.get('type') == 'group':
            walk(l.get('children', []), out)


def load(d, name):
    p = json.load(open(f'{d}/{name}.motion.json'))
    art = p['project'].get('art') or {}
    info = {'look': art.get('look'), 'pal_idx': art.get('palette'), 'plate': art.get('plate'),
            'bg': p['canvas']['background'], 'palette': p['theme'].get('palette'),
            'fonts': tuple(sorted(set(p['theme'].get('fonts', {}).values()))),
            'dur': p['project'].get('duration_seconds'),
            # (0.23, additive) ProjectMeta.direction when the compile wrote one (None today).
            'direction': p['project'].get('direction'), 'beats': []}
    for s in p['scenes']:
        if not s['id'].startswith('beat_'):
            continue
        ls = []
        walk(s['layers'], ls)
        pref = s['id'].replace('beat_', 'b') + '.'
        ids = set(re.sub(r'\d+', '#', l['id'].replace(pref, '', 1)) for l in ls)
        ops = collections.Counter(m['op'] for m in s['motions'])
        eas = collections.Counter('spring' if m.get('spring') else m.get('easing', 'linear')
                                  for m in s['motions'])
        cam = [m['op'] for m in (s.get('camera') or {}).get('motions', [])]
        info['beats'].append(dict(id=s['id'], dur=s['duration_seconds'], ids=ids, ops=ops, eas=eas,
                                  cam=cam, persp=bool((s.get('camera') or {}).get('perspective')),
                                  post=[x['effect'] for x in s.get('post', [])],
                                  nl=len(ls), nm=len(s['motions'])))
    return info


def jacc(a, b):
    return 1 - len(a & b) / max(1, len(a | b))


def opdist(a, b):
    keys = set(a) | set(b)
    return sum(abs(a[k] - b[k]) for k in keys) / max(1, sum(a.values()) + sum(b.values()))


def compare(d, n1, n2):
    a, b = load(d, n1), load(d, n2)
    rows = [(round(jacc(x['ids'], y['ids']), 2), round(opdist(x['ops'], y['ops']), 2),
             x['cam'] == y['cam'], round(x['dur'] - y['dur'], 2))
            for x, y in zip(a['beats'], b['beats'])]
    head = dict(look=(a['look'], b['look']), palette_same=a['palette'] == b['palette'],
                fonts_same=a['fonts'] == b['fonts'], bg=(a['bg'], b['bg']), plate=(a['plate'], b['plate']))
    return head, rows


def summary(d, name):
    a = load(d, name)
    print(f"## {name}: look={a['look']} pal={a['pal_idx']} plate={a['plate']} bg={a['bg']} dur={a['dur']}")
    for b in a['beats']:
        ops = ','.join(f'{k}:{v}' for k, v in b['ops'].most_common(6))
        print(f"  {b['id']} {b['dur']:.2f}s layers={b['nl']} motions={b['nm']} persp={b['persp']} "
              f"cam={sorted(set(b['cam']))} post={sorted(set(b['post']))}\n     ops[{ops}]")


if __name__ == '__main__':
    d, mode = sys.argv[1], sys.argv[2]
    if mode == 'sum':
        for n in sys.argv[3:]:
            summary(d, n)
    else:
        base = sys.argv[3]
        for n in sys.argv[4:]:
            head, rows = compare(d, base, n)
            print(f'{base} vs {n}: {head}\n   per-beat: {rows}')
