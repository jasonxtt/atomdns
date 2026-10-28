"""Conservative functional trace comparator; never authorizes timing runs."""
import datetime
import json

TIME_FIELDS = {'first_confirmed', 'last_confirmed', 'expires_at'}


def epoch(value):
    return datetime.datetime.fromisoformat(value.replace('Z', '+00:00')).timestamp()


def state(rows):
    # Preserve category, name, mark, reason, hits and unknown future fields.
    return sorted((dict((k, v) for k, v in row.items() if k not in TIME_FIELDS)
                   for row in rows), key=lambda r: json.dumps(r, sort_keys=True))


def compare(base, candidate, planned, initial_base=(), initial_candidate=(), margin=2):
    reasons = []
    for role, rows, initial in [('base', base, initial_base), ('candidate', candidate, initial_candidate)]:
        try:
            if planned <= 0 or len(rows) != planned or [r['index'] for r in rows] != list(range(planned)):
                reasons.append(role + ': missing or out-of-order records')
            if not rows:
                continue
            start, end = rows[0]['started'], rows[-1]['finished']
            if end < start or any(r['finished'] < r['started'] or not r['dns'] or not isinstance(r['route'], dict) for r in rows):
                reasons.append(role + ': invalid observation')
            entries = list(initial) + [e for r in rows for phase in ['before', 'after'] for e in r[phase]]
            if any(epoch(e['expires_at']) <= end + margin for e in entries):
                reasons.append(role + ': observed live entry expires within trace safety horizon')
            if any(r['started'] < rows[i-1]['finished'] for i, r in enumerate(rows) if i):
                reasons.append(role + ': observations overlap or wall clock moved backwards')
        except (KeyError, TypeError, ValueError):
            reasons.append(role + ': malformed trace')
    if reasons:
        return dict(status='incomplete', reasons=reasons, whole_project_accepted=False, timing_eligible=False)
    differences = []
    if state(initial_base) != state(initial_candidate):
        differences.append(dict(index=None, field='initial_learner'))
    for index, (old, new) in enumerate(zip(base, candidate)):
        for key in ['before', 'after', 'dns', 'route']:
            a, b = old[key], new[key]
            if key in {'before', 'after'}:
                a, b = state(a), state(b)
            if a != b:
                differences.append(dict(index=index, field=key))
    return dict(status='fail' if differences else 'pass', differences=differences,
                whole_project_accepted=False, timing_eligible=False)
