import assert from 'node:assert/strict';
import test from 'node:test';
import { recordObservation, renderChart, renderPlaceholder, validateHistory, validateRepository, publishHistory } from '../scripts/update-star-history.mjs';

const REPOSITORY = 'example/SafeBrowse';
const CREATED_AT = '2026-09-05T09:00:00Z';
const FIRST_OBSERVATION = '2026-09-05T10:00:00.000Z';
const OLD_REVISION = '1'.repeat(40);
const OLD_TREE = '2'.repeat(40);
const DATA_BLOB = '3'.repeat(40);
const NEW_TREE = '4'.repeat(40);
const NEW_REVISION = '5'.repeat(40);

function firstObservation(stars = 0) {
    return recordObservation(null, REPOSITORY, CREATED_AT, stars, FIRST_OBSERVATION);
}

/** Fake GitHub responses exercise publication without any network or credentials. */
function fakeGithub(previous, { stars = 0, rejectRefUpdate = false, unexpectedFiles = false } = {}) {
    const calls = [];
    const request = async (url, options) => {
        const path = url.replace(`https://api.github.com/repos/${REPOSITORY}`, '');
        const body = options.body ? JSON.parse(options.body) : undefined;
        calls.push({ path, method: options.method, body });
        let status = 200;
        let value;
        if (path === '') value = { full_name: REPOSITORY, private: false, created_at: CREATED_AT, stargazers_count: stars };
        else if (path === '/git/ref/heads/star-history') {
            status = previous ? 200 : 404;
            value = { object: { sha: OLD_REVISION } };
        } else if (path === `/git/commits/${OLD_REVISION}`) value = { tree: { sha: OLD_TREE } };
        else if (path === `/git/trees/${OLD_TREE}?recursive=1`) value = { truncated: false, tree: [
            { path: 'stars.json', type: 'blob', mode: '100644', sha: DATA_BLOB, size: Buffer.byteLength(JSON.stringify(previous)) },
            { path: unexpectedFiles ? 'unrelated.txt' : 'star-history.svg', type: 'blob', mode: '100644', sha: DATA_BLOB, size: 100 },
        ] };
        else if (path === `/git/blobs/${DATA_BLOB}`) value = { encoding: 'base64', content: Buffer.from(JSON.stringify(previous)).toString('base64') };
        else if (path === '/git/trees' && options.method === 'POST') value = { sha: NEW_TREE };
        else if (path === '/git/commits' && options.method === 'POST') value = { sha: NEW_REVISION };
        else if (path === '/git/refs' || path === '/git/refs/heads/star-history') {
            status = rejectRefUpdate ? 422 : 201;
            value = {};
        } else throw new Error(`Unexpected fixture request: ${options.method} ${path}`);
        return { status, ok: status >= 200 && status < 300, json: async () => value };
    };
    return { calls, request };
}

test('a zero-star first observation is real, and the pending graphic invents no count', () => {
    const history = firstObservation();
    assert.deepEqual(history.samples, [{ date: '2026-09-05', stars: 0 }]);
    const svg = renderChart(history);
    assert.match(svg, /Just launched/);
    assert.match(svg, /0 stars/);
    assert.match(svg, /First observation/);
    assert.doesNotMatch(svg, /polyline/);
    assert.doesNotMatch(svg, /NaN|Infinity|<script|href=/);
    const pending = renderPlaceholder(REPOSITORY);
    assert.match(pending, /No star counts have been recorded yet/);
    assert.doesNotMatch(pending, /0 stars|<circle|polyline/);
});

test('daily snapshots replace same-day totals, allow falling totals, and never backfill missed days', () => {
    const first = firstObservation(10);
    const changed = recordObservation(first, REPOSITORY, CREATED_AT, 8, '2026-09-05T12:00:00Z');
    const later = recordObservation(changed, REPOSITORY, CREATED_AT, 11, '2026-09-08T12:00:00Z');
    assert.deepEqual(first.samples, [{ date: '2026-09-05', stars: 10 }]);
    assert.deepEqual(later.samples, [{ date: '2026-09-05', stars: 8 }, { date: '2026-09-08', stars: 11 }]);
    assert.equal(recordObservation(later, REPOSITORY, CREATED_AT, 11, '2026-09-08T13:00:00Z'), later);
    const svg = renderChart(later);
    assert.match(svg, /polyline/);
    assert.match(svg, /11 stars/);
    assert.match(svg, /Missed days are not backfilled/);
    assert.doesNotMatch(svg, /NaN|Infinity/);
});

test('foreign, malformed, duplicate, negative, and out-of-order data fail closed', () => {
    const history = firstObservation();
    assert.throws(() => validateRepository('example/../private'));
    assert.throws(() => validateRepository('example/<script>'));
    assert.throws(() => validateHistory(history, 'other/repository'));
    assert.throws(() => validateHistory({ ...history, samples: [...history.samples, history.samples[0]] }));
    assert.throws(() => validateHistory({ ...history, samples: [{ date: '2026-02-30', stars: 0 }] }));
    assert.throws(() => recordObservation(history, REPOSITORY, CREATED_AT, -1, FIRST_OBSERVATION));
    assert.throws(() => recordObservation(history, REPOSITORY, CREATED_AT, 1.5, FIRST_OBSERVATION));
    assert.throws(() => recordObservation(history, REPOSITORY, CREATED_AT, 1, '2026-09-05T09:30:00Z'));
    assert.throws(() => recordObservation(history, REPOSITORY, '2026-09-05T09:01:00Z', 1, FIRST_OBSERVATION));
});

test('initial publication creates an orphan branch containing only the two data files', async () => {
    const github = fakeGithub(null);
    await publishHistory({ repository: REPOSITORY, token: 'fixture-only', request: github.request, now: new Date(FIRST_OBSERVATION) });
    const tree = github.calls.find(call => call.path === '/git/trees').body;
    assert.deepEqual(tree.tree.map(entry => entry.path), ['stars.json', 'star-history.svg']);
    assert.equal(tree.base_tree, undefined);
    assert.deepEqual(github.calls.find(call => call.path === '/git/commits').body.parents, []);
    assert.deepEqual(github.calls.at(-1), { path: '/git/refs', method: 'POST', body: { ref: 'refs/heads/star-history', sha: NEW_REVISION } });
});

test('an unchanged same-day observation performs no Git mutations', async () => {
    const github = fakeGithub(firstObservation());
    const result = await publishHistory({ repository: REPOSITORY, token: 'fixture-only', request: github.request, now: new Date('2026-09-05T11:00:00Z') });
    assert.equal(result.changed, false);
    assert.ok(github.calls.every(call => call.method === 'GET'));
});

test('concurrent history updates fail without a forced overwrite or fallback', async () => {
    const github = fakeGithub(firstObservation(), { stars: 2, rejectRefUpdate: true });
    await assert.rejects(publishHistory({ repository: REPOSITORY, token: 'fixture-only', request: github.request, now: new Date('2026-09-06T10:00:00Z') }), /422/);
    assert.equal(github.calls.find(call => call.path === '/git/trees').body.base_tree, OLD_TREE);
    assert.deepEqual(github.calls.find(call => call.path === '/git/commits').body.parents, [OLD_REVISION]);
    assert.deepEqual(github.calls.at(-1), { path: '/git/refs/heads/star-history', method: 'PATCH', body: { sha: NEW_REVISION, force: false } });
});

test('an existing branch with unrelated files is preserved', async () => {
    const github = fakeGithub(firstObservation(), { stars: 2, unexpectedFiles: true });
    await assert.rejects(publishHistory({ repository: REPOSITORY, token: 'fixture-only', request: github.request, now: new Date('2026-09-06T10:00:00Z') }), /unexpected files/);
    assert.ok(github.calls.every(call => call.method === 'GET'));
});
