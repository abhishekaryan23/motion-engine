#!/usr/bin/env python3
"""Sprint 0.23 variety bench: compile the benchmark matrix the way the product
path does and report structural variety, determinism, warnings and hard QA.

Run it through `scripts/variety_bench.sh` (builds the CLI, writes
`output/sprint23/bench/variety_report.json`, optional contact sheets).

Per-beat distance (FROZEN for sprint 0.23; every task and the final judgement
use this definition):

    D(a, b) = 0.5 * J(roles) + 0.25 * opdist(ops) + 0.25 * [camera ops differ]

`J`, `opdist` and the layer roles are exactly those of `metrics.py`:
  roles    layer ids with the beat prefix removed and digits replaced by '#'
  J        Jaccard distance of the two role sets, 1 - |A & B| / |A | B|
  opdist   sum over op names of |count_a - count_b| / (total ops a + total ops b)
  camera   the ordered list of the beat's camera motion ops (metrics' `cam`)
  [..]     1 when the two camera op lists differ, else 0

D is 0 for two beats built by the same template with different text and the
same camera, and 1 for beats with no shared role, no shared op and different
camera ops. Beats are paired by index (zip over the shorter list).

Matrix: 8 stories x 9 tones (a style file `{"tone": t}`) x the takes in TAKES,
plus the six mood styles of `examples/bench/styles/` for the two review
stories. Every compile is

    compile <intent> --style <s> --speech <fixture> --art auto --variety auto
            [--take N] --output <dir>/<story>__<tone>__t<N>.motion.json

followed by `qa <scene> --layout --json` (each finding is a hard FAIL; it includes
`text_local_contrast`) and by `qa <scene> --speech <fixture> --json`, which gives the four
0.23 checks `text_local_contrast`, `dead_air`, `count_unsettled` and `repeat_template`
(FAIL of `dead_air` / `count_unsettled` is a hard finding too; WARN of `repeat_template` /
`count_unsettled` goes to `warn_qa`; `qa_checks` counts every status per check and lists
every FAIL / WARN without a cap).
`--take N` is passed only for N > 0 and only when `compile --help` lists
`--take`; otherwise the report says `"takes": "unavailable"` and runs take 0.
`--speech tight` (script: `SPEECH=tight`) reads `golden/fixtures/variety/tight/` instead: the
same eight speech maps with every gap between sentences set to 0.30 s, a continuous take
(`tighten_speech.py`); the report records `"speech": "tight"`.

Report keys: matrix, determinism, take_distance, cross_story, repeats,
tone_distinctness, warnings, hard_qa, warn_qa, qa_checks, timing (plus summary, takes, commit).

Repeat rule: two consecutive beats repeat when (template, entrance, camera
move) are equal in the direction record (`ProjectMeta.direction`, beats[]
.template / .params.entrance / .params.camera, "builder" when absent) and at
least one of the two beats had `alternates > 1`, so an alternative existed.
Without a direction record (every compile before the direction planner) the
approximation is J(roles) = 0 and equal camera ops, flagged `approx`.
"""
import argparse
import collections
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..', '..', '..'))
sys.dont_write_bytecode = True  # no __pycache__ beside the tools
sys.path.insert(0, HERE)
import metrics  # noqa: E402

STORIES = [
    ('sleep_review', 'examples/bench/stories/sleep_review.intent.json'),
    ('money_review', 'examples/bench/stories/money_review.intent.json'),
    ('collection-accumulate', 'examples/public/collection-accumulate.intent.json'),
    ('state-change', 'examples/public/state-change.intent.json'),
    ('derived-metric', 'examples/public/derived-metric.intent.json'),
    ('layers', 'examples/public/layers.intent.json'),
    ('ai_learns', 'examples/topics/ai_learns.intent.json'),
    ('space', 'examples/cinematic/space.intent.json'),
]
TONES = ['auto', 'editorial', 'technical', 'playful', 'street', 'documentary', 'hype',
         'studio', 'cinematic']
MOOD_STORIES = ['sleep_review', 'money_review']
MOODS = ['calm', 'cinematic', 'corporate', 'documentary', 'energetic', 'playful']
MOOD_DIR = 'examples/bench/styles'
FIXTURE_DIR = 'golden/fixtures/variety'
# (0.23 A4-short) `--speech tight`: the continuous-take fixtures, every gap between
# sentences 0.30 s (made by tighten_speech.py); `--speech default`: the `say` fixtures.
SPEECH_DIRS = {'default': FIXTURE_DIR, 'tight': FIXTURE_DIR + '/tight'}
CROSS_A, CROSS_B = 'sleep_review', 'money_review'
DETERMINISM = ('sleep_review', 'playful', 3)
SHEET_STORIES = ['sleep_review', 'money_review', 'ai_learns']
SHEET_TONES = ['playful', 'cinematic', 'documentary', 'editorial']
FINDINGS_CAP = 80
# (0.23 W4) the checks `qa --speech --json` adds; hard when FAIL, `warn_qa` when WARN.
NEW_CHECKS = ['text_local_contrast', 'dead_air', 'count_unsettled', 'repeat_template', 'carry_continuity']
# text_local_contrast is already a layout finding (`qa --layout`); its speech entry is
# counted in `qa_checks` only, so it is not a second hard finding.
HARD_FROM_SPEECH = ['dead_air', 'count_unsettled', 'carry_continuity']

WARNING_RE = re.compile(r'^warning\[([A-Za-z0-9_]+)\]')


# ---------------------------------------------------------------------------
# distance
# ---------------------------------------------------------------------------

def beat_distance(x, y):
    """D(a, b) for two beats of `metrics.load` (see the module docstring)."""
    j = metrics.jacc(x['ids'], y['ids'])
    o = metrics.opdist(x['ops'], y['ops'])
    c = 0.0 if x['cam'] == y['cam'] else 1.0
    return 0.5 * j + 0.25 * o + 0.25 * c


def distances(a, b):
    return [round(beat_distance(x, y), 4) for x, y in zip(a['beats'], b['beats'])]


def mean(xs):
    return round(sum(xs) / len(xs), 4) if xs else None


# ---------------------------------------------------------------------------
# direction record (absent before the planner exists)
# ---------------------------------------------------------------------------

def direction_beats(info):
    d = info.get('direction')
    if not isinstance(d, dict):
        return None
    beats = d.get('beats')
    return beats if isinstance(beats, list) and beats else None


def dir_key(b):
    p = b.get('params') or {}
    return (b.get('template'), p.get('entrance', 'builder'), p.get('camera', 'builder'))


def changed_beats(a, b):
    """Beat indices whose template or entrance differ between two compiles,
    or None when either has no direction record."""
    da, db = direction_beats(a), direction_beats(b)
    if da is None or db is None:
        return None
    out = []
    for i, (x, y) in enumerate(zip(da, db)):
        ex = (x.get('params') or {}).get('entrance', 'builder')
        ey = (y.get('params') or {}).get('entrance', 'builder')
        if x.get('template') != y.get('template') or ex != ey:
            out.append(i)
    return out


def repeat_pairs(info):
    """(pairs, approx): consecutive beat index pairs that repeat."""
    beats = direction_beats(info)
    pairs = []
    if beats is not None:
        for i in range(len(beats) - 1):
            x, y = beats[i], beats[i + 1]
            alt = max(int(x.get('alternates', 1)), int(y.get('alternates', 1))) > 1
            if alt and dir_key(x) == dir_key(y):
                pairs.append([i, i + 1])
        return pairs, False
    bs = info['beats']
    for i in range(len(bs) - 1):
        x, y = bs[i], bs[i + 1]
        if metrics.jacc(x['ids'], y['ids']) == 0 and x['cam'] == y['cam']:
            pairs.append([i, i + 1])
    return pairs, True


# ---------------------------------------------------------------------------
# running the engine
# ---------------------------------------------------------------------------

def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, cwd=ROOT)


def takes_flag_available(engine):
    r = run([engine, 'compile', '--help'])
    return '--take' in (r.stdout + r.stderr)


def compile_scene(engine, intent, style, speech, out, take):
    cmd = [engine, 'compile', intent, '--style', style, '--speech', speech,
           '--art', 'auto', '--variety', 'auto']
    if take > 0:
        cmd += ['--take', str(take)]
    cmd += ['--output', out]
    t0 = time.perf_counter()
    r = run(cmd)
    ms = (time.perf_counter() - t0) * 1000.0
    codes = [m.group(1) for line in r.stdout.splitlines() if (m := WARNING_RE.match(line))]
    return r.returncode, ms, codes, (r.stderr or r.stdout).strip()[-400:]


def layout_qa(engine, scene):
    r = run([engine, 'qa', scene, '--layout', '--json'])
    if r.returncode != 0:
        return None, r.stderr.strip()[-300:]
    try:
        return json.loads(r.stdout), None
    except json.JSONDecodeError as e:
        return None, f'bad qa json: {e}'


def speech_checks(engine, scene, speech):
    """The four 0.23 checks of `qa <scene> --speech <fixture> --json` -> ({name: check}, error)."""
    r = run([engine, 'qa', scene, '--speech', speech, '--json'])
    if r.returncode != 0:
        return None, r.stderr.strip()[-300:]
    try:
        rep = json.loads(r.stdout)
    except json.JSONDecodeError as e:
        return None, f'bad qa json: {e}'
    return {c['name']: c for c in rep.get('checks', []) if c['name'] in NEW_CHECKS}, None


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        h.update(f.read())
    return h.hexdigest()


def git_commit():
    r = run(['git', 'rev-parse', '--short', 'HEAD'])
    return r.stdout.strip() if r.returncode == 0 else None


# ---------------------------------------------------------------------------
# the matrix
# ---------------------------------------------------------------------------

def name_of(story, tone, take):
    return f'{story}__{tone}__t{take}'


def build_variants(style_dir):
    """story -> [(label, kind, style path)]"""
    tone_styles = []
    for t in TONES:
        p = os.path.join(style_dir, f'{t}.style.json')
        with open(p, 'w') as f:
            json.dump({'tone': t}, f)
        tone_styles.append((t, 'tone', p))
    mood_styles = [(f'mood_{m}', 'mood', os.path.join(ROOT, MOOD_DIR, f'{m}.style.json'))
                   for m in MOODS]
    return {s: tone_styles + (mood_styles if s in MOOD_STORIES else []) for s, _ in STORIES}


def run_matrix(engine, out, takes, style_dir):
    variants = build_variants(style_dir)
    matrix, infos, qa_findings, warn_findings, check_list = [], {}, [], [], []
    for story, intent in STORIES:
        speech = os.path.join(FIXTURE_DIR, f'{story}.speech.json')
        if not os.path.isfile(os.path.join(ROOT, speech)):
            raise SystemExit(f'missing speech fixture {speech} '
                             '(regenerate: REGEN_FIXTURES=1 scripts/variety_bench.sh)')
        for label, kind, style in variants[story]:
            for take in takes:
                name = name_of(story, label, take)
                scene = os.path.join(out, name + '.motion.json')
                rc, ms, codes, tail = compile_scene(engine, intent, style, speech, scene, take)
                row = {'story': story, 'tone': label, 'kind': kind, 'take': take,
                       'ok': rc == 0, 'look': None, 'palette': None, 'ground': None,
                       'plate': None, 'fonts': [], 'beats': 0, 'warnings': codes,
                       'hard_qa': None, 'compile_ms': round(ms, 1)}
                if rc != 0:
                    row['error'] = tail
                    matrix.append(row)
                    continue
                info = metrics.load(out, name)
                infos[(story, label, take)] = info
                row.update(look=info['look'], palette=info['pal_idx'], ground=info['bg'],
                           plate=info['plate'], fonts=list(info['fonts']),
                           beats=len(info['beats']))
                report, err = layout_qa(engine, scene)
                if report is None:
                    row['ok'] = False
                    row['error'] = err
                else:
                    found = report.get('findings', [])
                    row['hard_qa'] = len(found)
                    for f in found:
                        qa_findings.append({'story': story, 'tone': label, 'take': take,
                                            'scene': f.get('scene'), 'layer': f.get('layer'),
                                            'check': f.get('check'), 'detail': f.get('detail')})
                t0 = time.perf_counter()
                checks, err = speech_checks(engine, scene, speech)
                row['qa_ms'] = round((time.perf_counter() - t0) * 1000.0, 1)
                if checks is None:
                    row['ok'] = False
                    row['error'] = err
                else:
                    row['qa_checks'] = {n: c['status'] for n, c in checks.items()}
                    for n, c in checks.items():
                        item = {'story': story, 'tone': label, 'take': take, 'check': n,
                                'status': c['status'], 'detail': c['detail']}
                        if c['status'] in ('fail', 'warn'):
                            check_list.append(item)
                        if c['status'] == 'fail' and n in HARD_FROM_SPEECH:
                            qa_findings.append({'story': story, 'tone': label, 'take': take,
                                                'scene': None, 'layer': None, 'check': n,
                                                'detail': c['detail']})
                        elif c['status'] == 'warn':
                            warn_findings.append({'story': story, 'tone': label, 'take': take,
                                                  'scene': None, 'layer': None, 'check': n,
                                                  'detail': c['detail']})
                matrix.append(row)
    return matrix, infos, qa_findings, warn_findings, check_list


def run_determinism(engine, out, style_dir):
    story, tone, runs = DETERMINISM
    intent = dict(STORIES)[story]
    style = os.path.join(style_dir, f'{tone}.style.json')
    speech = os.path.join(FIXTURE_DIR, f'{story}.speech.json')
    scene = os.path.join(out, '_determinism.motion.json')
    digests = []
    for _ in range(runs):
        rc, _, _, tail = compile_scene(engine, intent, style, speech, scene, 0)
        digests.append(sha256(os.path.join(ROOT, scene)) if rc == 0 else f'error: {tail}')
    return {'story': story, 'tone': tone, 'compiles': runs, 'sha256': digests,
            'equal': len(set(digests)) == 1 and not digests[0].startswith('error')}


def take_distance(infos, takes):
    rows = []
    for story, _ in STORIES:
        labels = sorted({k[1] for k in infos if k[0] == story}, key=lambda t: (
            TONES.index(t) if t in TONES else 100 + MOODS.index(t[5:])))
        for label in labels:
            base = infos.get((story, label, 0))
            if base is None:
                continue
            for take in takes:
                other = infos.get((story, label, take))
                if take == 0 or other is None:
                    continue
                per = distances(base, other)
                changed = changed_beats(base, other)
                rows.append({'story': story, 'tone': label, 'take': take, 'mean_d': mean(per),
                             'per_beat_d': per,
                             'changed_beats': None if changed is None else len(changed),
                             'changed_beat_indices': changed})
    return rows


def cross_story(infos):
    rows = []
    for tone in TONES:
        a, b = infos.get((CROSS_A, tone, 0)), infos.get((CROSS_B, tone, 0))
        if a is None or b is None:
            continue
        per = distances(a, b)
        # Plan section 0 quotes the role distance alone ("0.0 on 4 of 5 beats"):
        # keep J(roles) per beat beside the composite D.
        per_j = [round(metrics.jacc(x['ids'], y['ids']), 4)
                 for x, y in zip(a['beats'], b['beats'])]
        rows.append({'tone': tone, 'per_beat_d': per, 'mean_d': mean(per),
                     'zero_beats': sum(1 for d in per if d == 0),
                     'per_beat_j': per_j, 'j_zero_beats': sum(1 for j in per_j if j == 0),
                     'look': [a['look'], b['look']],
                     'same_palette': a['palette'] == b['palette'],
                     'same_fonts': a['fonts'] == b['fonts'],
                     'same_ground': a['bg'] == b['bg'], 'same_plate': a['plate'] == b['plate']})
    return rows


def repeats(infos):
    rows = []
    for (story, tone, take), info in infos.items():
        pairs, approx = repeat_pairs(info)
        rows.append({'story': story, 'tone': tone, 'take': take, 'approx': approx,
                     'pairs': pairs, 'count': len(pairs)})
    return rows


def same_look_palette_ground(a, b):
    return a['look'] == b['look'] and a['palette'] == b['palette'] and a['bg'] == b['bg']


def equal_pairs(infos, story, labels):
    """Pairs among `labels` (take 0) with equal look + palette + ground."""
    have = [t for t in labels if (story, t, 0) in infos]
    out = []
    for i, t1 in enumerate(have):
        for t2 in have[i + 1:]:
            a, b = infos[(story, t1, 0)], infos[(story, t2, 0)]
            if same_look_palette_ground(a, b):
                out.append({'a': t1, 'b': t2, 'look': a['look'], 'ground': a['bg'],
                            'same_fonts': a['fonts'] == b['fonts'],
                            'same_plate': a['plate'] == b['plate']})
    return have, out


def tone_distinctness(infos):
    per_story = {}
    for story, _ in STORIES:
        have, equal = equal_pairs(infos, story, TONES)
        entry = {
            'distinct_looks': len({infos[(story, t, 0)]['look'] for t in have}),
            'tones': len(have),
            'looks': {t: infos[(story, t, 0)]['look'] for t in have},
            'equal_look_palette_ground': equal}
        if story in MOOD_STORIES:
            moods = [f'mood_{m}' for m in MOODS]
            have_m, equal_m = equal_pairs(infos, story, moods)
            entry['moods'] = {t: infos[(story, t, 0)]['look'] for t in have_m}
            entry['mood_equal_look_palette_ground'] = equal_m
        per_story[story] = entry
    pairs = []
    for story in MOOD_STORIES:
        a, b = infos.get((story, 'mood_calm', 0)), infos.get((story, 'mood_corporate', 0))
        if a is None or b is None:
            continue
        pairs.append({'story': story, 'a': 'calm', 'b': 'corporate',
                      'look': [a['look'], b['look']], 'ground': [a['bg'], b['bg']],
                      'same_look': a['look'] == b['look'],
                      'same_ground': a['bg'] == b['bg'],
                      'same_palette': a['palette'] == b['palette'],
                      'same_fonts': a['fonts'] == b['fonts'],
                      'same_look_and_ground': a['look'] == b['look'] and a['bg'] == b['bg']})
    return {'per_story': per_story, 'calm_vs_corporate': pairs}


def build_sheets(engine, out, takes, sheet_dir):
    os.makedirs(os.path.join(ROOT, sheet_dir), exist_ok=True)
    written, errors = [], []
    for story in SHEET_STORIES:
        for tone in SHEET_TONES:
            png = os.path.join(sheet_dir, f'{story}__{tone}.png')
            names = [name_of(story, tone, t) for t in takes]
            r = run([sys.executable, os.path.join(HERE, 'sheet.py'), out, engine, png] + names)
            if r.returncode == 0:
                written.append(png)
            else:
                errors.append({'sheet': png, 'error': (r.stderr or r.stdout).strip()[-300:]})
    return written, errors


def parse_takes(text):
    out = []
    for tok in re.split(r'[,\s]+', text.strip()):
        if tok:
            out.append(int(tok))
    return sorted(set(out)) or [0]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--engine', default='target/release/motion-engine')
    ap.add_argument('--out', default='output/sprint23/bench')
    ap.add_argument('--takes', default='0')
    ap.add_argument('--sheets', action='store_true')
    ap.add_argument('--strict', action='store_true',
                    help='exit 1 on any compile failure, hard QA finding, non-deterministic '
                         'compile or failed sheet (default: report and exit 0)')
    ap.add_argument('--list-stories', action='store_true',
                    help='print "name<TAB>intent path" for each story and exit')
    ap.add_argument('--speech', choices=sorted(SPEECH_DIRS), default='default',
                    help='speech fixtures: default (the say fixtures) or tight (every gap '
                         'between sentences 0.30 s, a continuous take)')
    args = ap.parse_args()
    if args.list_stories:
        for name, path in STORIES:
            print(f'{name}\t{path}')
        return 0
    global FIXTURE_DIR
    FIXTURE_DIR = SPEECH_DIRS[args.speech]

    os.chdir(ROOT)  # every path below is relative to the repo root
    engine = args.engine if os.path.isabs(args.engine) else os.path.join(ROOT, args.engine)
    if not os.path.isfile(engine):
        raise SystemExit(f'no engine at {engine} (cargo build --release -p motion-cli)')
    out = args.out
    os.makedirs(os.path.join(ROOT, out), exist_ok=True)

    requested = parse_takes(args.takes)
    flag = takes_flag_available(engine)
    takes = requested if flag else [0]
    takes_field = takes if flag else 'unavailable'

    started = time.perf_counter()
    with tempfile.TemporaryDirectory(prefix='variety_styles_') as style_dir:
        matrix, infos, findings, warn_findings, check_list = run_matrix(
            engine, out, takes, style_dir)
        determinism = run_determinism(engine, out, style_dir)
    compile_s = sum(r['compile_ms'] for r in matrix) / 1000.0

    take_rows = take_distance(infos, takes)
    cross = cross_story(infos)
    rep = repeats(infos)
    tone = tone_distinctness(infos)

    warn_counts = collections.Counter(c for r in matrix for c in r['warnings'])
    per_check = collections.Counter(f['check'] for f in findings)
    hard = {'total': len(findings), 'per_check': dict(sorted(per_check.items())),
            'compiles_with_findings': sum(1 for r in matrix if r['hard_qa']),
            'findings': findings[:FINDINGS_CAP], 'findings_truncated': len(findings) > FINDINGS_CAP}
    warn_per_check = collections.Counter(f['check'] for f in warn_findings)
    warn = {'total': len(warn_findings), 'per_check': dict(sorted(warn_per_check.items())),
            'findings': warn_findings[:FINDINGS_CAP],
            'findings_truncated': len(warn_findings) > FINDINGS_CAP}
    status_counts = {n: collections.Counter(r['qa_checks'][n] for r in matrix
                                            if r.get('qa_checks') and n in r['qa_checks'])
                     for n in NEW_CHECKS}
    qa_checks = {'per_check': {n: {k: status_counts[n].get(k, 0)
                                   for k in ('pass', 'warn', 'fail', 'skip')}
                               for n in NEW_CHECKS},
                 'fail_list': [f for f in check_list if f['status'] == 'fail'],
                 'warn_list': [f for f in check_list if f['status'] == 'warn'],
                 'qa_seconds': round(sum(r.get('qa_ms') or 0 for r in matrix) / 1000.0, 2)}
    total_s = time.perf_counter() - started

    report = {
        'commit': git_commit(),
        'speech': args.speech,
        'takes': takes_field,
        'takes_run': takes,
        'stories': [s for s, _ in STORIES],
        'tones': TONES,
        'matrix': matrix,
        'determinism': determinism,
        'take_distance': take_rows,
        'cross_story': cross,
        'repeats': rep,
        'tone_distinctness': tone,
        'warnings': dict(sorted(warn_counts.items())),
        'hard_qa': hard,
        'warn_qa': warn,
        'qa_checks': qa_checks,
        'timing': {'total_seconds': round(total_s, 2), 'compiles': len(matrix),
                   'compile_seconds': round(compile_s, 2)},
        'summary': {
            'compile_failures': sum(1 for r in matrix if not r['ok']),
            'cross_story_mean_d': {r['tone']: r['mean_d'] for r in cross},
            'cross_story_zero_beats': {r['tone']: r['zero_beats'] for r in cross},
            'take_mean_d': mean([r['mean_d'] for r in take_rows if r['mean_d'] is not None]),
            'repeat_pairs': sum(r['count'] for r in rep),
            'repeat_pairs_approx_compiles': sum(1 for r in rep if r['approx']),
            'tone_collapse_pairs': {s: [[p['a'], p['b']] for p in v['equal_look_palette_ground']]
                                    for s, v in tone['per_story'].items()},
            'mood_collapse_pairs': {s: [[p['a'], p['b']]
                                        for p in v['mood_equal_look_palette_ground']]
                                    for s, v in tone['per_story'].items()
                                    if 'mood_equal_look_palette_ground' in v},
            'calm_corporate_same_look_and_ground': [p['same_look_and_ground']
                                                    for p in tone['calm_vs_corporate']],
            'hard_qa_total': len(findings),
            'warn_qa_total': len(warn_findings),
            'warning_total': sum(warn_counts.values()),
        },
    }

    if args.sheets:
        t0 = time.perf_counter()
        written, errors = build_sheets(engine, out, takes, os.path.join(out, 'sheets'))
        report['sheets'] = {'written': written, 'errors': errors,
                            'seconds': round(time.perf_counter() - t0, 2)}

    path = os.path.join(ROOT, out, 'variety_report.json')
    with open(path, 'w') as f:
        json.dump(report, f, indent=2)
        f.write('\n')
    print_summary(report, path)
    # A measurement run exits 0 once the report is written; --strict turns any
    # compile failure, hard QA finding, non-deterministic compile or failed
    # sheet into exit 1 (a gate for later tasks).
    problems = []
    if report['summary']['compile_failures']:
        problems.append(f"{report['summary']['compile_failures']} compile failure(s)")
    if findings:
        problems.append(f'{len(findings)} hard layout-QA finding(s)')
    if not determinism['equal']:
        problems.append('compile is not deterministic')
    if args.sheets and report['sheets']['errors']:
        problems.append(f"{len(report['sheets']['errors'])} sheet(s) failed")
    for p in problems:
        print(f"{'FAIL' if args.strict else 'note'}: {p}")
    return 1 if (problems and args.strict) else 0


def print_summary(report, path):
    s = report['summary']
    print(f"wrote {path}")
    print(f"compiles {report['timing']['compiles']} in {report['timing']['total_seconds']} s "
          f"(failures {s['compile_failures']}); takes {report['takes']}; "
          f"speech {report['speech']}")
    print(f"determinism equal={report['determinism']['equal']}")
    print('cross-story mean D / zero beats per tone (J = role distance alone):')
    for r in report['cross_story']:
        print(f"  {r['tone']:<12} mean {r['mean_d']}  per-beat {r['per_beat_d']}  zero {r['zero_beats']}"
              f"  | J {r['per_beat_j']} zero {r['j_zero_beats']}")
    print('tone collapse (equal look+palette+ground):')
    for story, pairs in s['tone_collapse_pairs'].items():
        if pairs:
            print(f'  {story}: {pairs}')
    for story, pairs in s['mood_collapse_pairs'].items():
        if pairs:
            print(f'  {story} (mood styles): {pairs}')
    print(f"calm vs corporate same look+ground: {s['calm_corporate_same_look_and_ground']}")
    print(f"warnings {report['warnings']}")
    print(f"hard QA {report['hard_qa']['total']} {report['hard_qa']['per_check']}")
    print(f"warn QA {report['warn_qa']['total']} {report['warn_qa']['per_check']}")
    print('0.23 checks (pass/warn/fail/skip per check):')
    for n, c in report['qa_checks']['per_check'].items():
        print(f"  {n:<20} {c['pass']}/{c['warn']}/{c['fail']}/{c['skip']}")


if __name__ == '__main__':
    sys.exit(main())
