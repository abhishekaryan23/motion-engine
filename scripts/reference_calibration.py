#!/usr/bin/env python3
"""Compare blind interpreter outputs with known 0.6 styles; never edit profiles."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
CASES = {
    'cal_1': ('dark_technical', {
        'tone': ['technical'], 'polarity': ['dark'], 'temperature': ['cool'],
        'typography_character': ['condensed', 'mono_technical'],
        'motion_temperament': ['snappy', 'mechanical'],
        'transition_character': ['geometric'],
    }),
    'cal_2': ('playful_print', {
        'tone': ['playful'], 'polarity': ['light'],
        'motion_temperament': ['energetic', 'playful'],
        'scale_contrast': ['large', 'dramatic'],
        'composition_rhythm': ['active', 'high_frequency'],
        'palette_character': ['multicolor', 'vivid'],
    }),
    'cal_3': ('warm_editorial', {
        'tone': ['editorial'], 'polarity': ['light'], 'temperature': ['warm'],
        'motion_temperament': ['restrained', 'fluid'],
        'material_character': ['paper', 'print'],
        'composition_rhythm': ['measured_editorial', 'slow_breathing'],
    }),
}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='target/release/motion-engine')
    parser.add_argument('--out', default='references/calibration/report.json')
    args = parser.parse_args()
    binary = ROOT / args.binary
    intent = ROOT / 'examples/editorial_demo.intent.json'
    def run(*argv):
        return json.loads(subprocess.check_output([str(binary), *map(str, argv)], cwd=ROOT, text=True))
    cases = []
    for opaque, (known, expected) in CASES.items():
        bundle = ROOT / 'references/calibration' / opaque / 'current'
        profile = bundle / 'reference-style.json'
        observed = json.loads(profile.read_text())
        run('validate-reference-style', profile, '--bundle', bundle, '--json')
        recovered = run('resolve-style', intent, '--reference-style', profile, '--json')['resolved']
        original = run('resolve-style', intent, '--style', ROOT / f'examples/taste/{known}.style.json', '--json')['resolved']
        checks = []
        for dimension, accepted in expected.items():
            trait = observed.get(dimension) or {}
            checks.append(dict(dimension=dimension, expected=accepted, observed=trait.get('value'),
                               confidence=trait.get('confidence'),
                               passed=trait.get('value') in accepted and trait.get('confidence', 0) >= .5))
        cases.append(dict(case=opaque, known_style=known, profile_sha256=hashlib.sha256(profile.read_bytes()).hexdigest(),
                          reference_fingerprint=observed.get('reference_fingerprint'), checks=checks,
                          passed=all(c['passed'] for c in checks), original=original, recovered=recovered))
    def fingerprint(r):
        return {k: r[k] for k in ['tone', 'background', 'typography', 'material', 'image_treatment', 'rhythm', 'scale', 'layers']} | {
            'polarity': r['palette']['polarity'], 'palette_family': r['palette']['family'],
            'motion': r['motion']['kind'], 'transition': r['transition']['family'], 'density': r['density']['level']}
    differences = []
    for i, a in enumerate(cases):
        for b in cases[i+1:]:
            x, y = fingerprint(a['recovered']), fingerprint(b['recovered'])
            dimensions = [k for k in x if x[k] != y[k]]
            differences.append(dict(a=a['case'], b=b['case'], differing_dimensions=dimensions))
    report = dict(passed=all(c['passed'] for c in cases) and all(d['differing_dimensions'] for d in differences),
                  interpretation='Blind external GPT-6 Luna; known styles withheld until evaluation.',
                  cases=cases, pairwise=differences)
    out = ROOT / args.out
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2) + '\n')
    for case in cases:
        print(case['known_style'], 'PASS' if case['passed'] else 'FAIL')
        for check in case['checks']:
            if not check['passed']:
                print(' ', check)
    raise SystemExit(0 if report['passed'] else 1)

if __name__ == '__main__':
    main()
