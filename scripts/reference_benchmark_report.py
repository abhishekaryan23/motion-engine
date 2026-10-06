#!/usr/bin/env python3
"""Record style differences and byte-identical intent use; no aesthetic score."""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]

def digest(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()

def dimensions(r):
    return {k: r[k] for k in ['background', 'typography', 'material', 'image_treatment', 'rhythm', 'scale', 'layers']} | {
        'polarity': r['palette']['polarity'], 'palette_family': r['palette']['family'],
        'motion': r['motion']['kind'], 'transition': r['transition']['family'], 'density': r['density']['level']}

def main():
    bundles = [Path(p).resolve() for p in sys.argv[1:]]
    if len(bundles) != 2:
        raise SystemExit('usage: reference_benchmark_report.py <bundle-a> <bundle-b>')
    cases = []
    for d in bundles:
        name = d.parent.name if d.name == 'current' else d.name
        cov = json.loads((d / 'coverage.json').read_text())
        ev = json.loads((d / 'evidence.json').read_text())
        video = d / f'{name}-styled.mp4'
        if not video.is_file() or not video.stat().st_size:
            raise SystemExit(f'Missing rendered video: {video}')
        cases.append(dict(name=name, bundle=str(d.relative_to(ROOT)),
                          reference_fingerprint=ev['reference_fingerprint'],
                          profile_sha256=digest(d / 'reference-style.json'),
                          intent_sha256=digest(d / 'work/intent.json'),
                          rendered_video=str(video.relative_to(ROOT)),
                          dimensions=dimensions(cov['resolved']), coverage=cov['coverage'],
                          qa=json.loads((d / 'qa.json').read_text())))
    a,b=cases
    same=a['intent_sha256']==b['intent_sha256']==digest(ROOT/'examples/demo_05/late_night_worker.intent.json')
    differences={k: {'a': a['dimensions'][k], 'b': b['dimensions'][k]}
                 for k in a['dimensions'] if a['dimensions'][k] != b['dimensions'][k]}
    temporal=any(k in differences for k in ['rhythm','layers','motion','transition'])
    visual=any(k in differences for k in ['background','typography','material','image_treatment','polarity','palette_family','density','scale'])
    result=dict(passed=same and temporal and visual, same_creative_intent=same,
                source_content_copied=False,
                asset_producer='Procedural fixture generator; target intent only, no reference assets.',
                differences=differences,cases=cases)
    (ROOT/'references/benchmark/report.json').write_text(json.dumps(result,indent=2)+'\n')
    print('benchmark', 'PASS' if result['passed'] else 'FAIL', 'differing dimensions:', ', '.join(differences))
    raise SystemExit(0 if result['passed'] else 1)

if __name__=='__main__':
    main()
