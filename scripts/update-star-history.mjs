/** Daily, observed GitHub star totals. Publishing is explicitly opt-in. */
import { readFile, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

const DATA_VERSION = 1;
const HISTORY_BRANCH = 'star-history';
const DATA_FILE = 'stars.json';
const CHART_FILE = 'star-history.svg';
const API_ORIGIN = 'https://api.github.com';
const API_VERSION = '2022-11-28';
const REQUEST_TIMEOUT_MS = 30_000;
const MAX_HISTORY_BYTES = 4 * 1024 * 1024;
const DAY_MS = 24 * 60 * 60 * 1000;
const PLOT = Object.freeze({ left: 64, top: 142, width: 832, height: 110 });

/** @typedef {{date: string, stars: number}} StarSample */
/** @typedef {{schemaVersion: number, repository: string, repositoryCreatedAt: string,
 * trackingStartedAt: string, updatedAt: string, samples: StarSample[]}} StarHistory */

/** Rejects paths and unrelated API targets before they enter GitHub requests. */
export function validateRepository(repository) {
    if (typeof repository !== 'string' ||
        !/^[A-Za-z0-9-]+\/[A-Za-z0-9_.-]+$/.test(repository) ||
        ['.', '..'].includes(repository.split('/')[1])) {
        throw new Error('Expected a GitHub repository in OWNER/REPOSITORY form.');
    }
    return repository;
}

/** Accepts only canonical UTC timestamps, avoiding locale-dependent date parsing. */
function validateTimestamp(value) {
    if (typeof value !== 'string' || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{3})?Z$/.test(value) ||
        !Number.isFinite(Date.parse(value)) || new Date(value).toISOString().replace('.000Z', 'Z') !== value.replace('.000Z', 'Z')) {
        throw new Error('Star history contains an invalid UTC timestamp.');
    }
    return value;
}

/** Fails closed on damaged, foreign, out-of-order or fabricated future history.
 * Time O(n), auxiliary space O(1), where n is the number of observed days. */
export function validateHistory(history, expectedRepository = history?.repository) {
    validateRepository(expectedRepository);
    if (!history || history.schemaVersion !== DATA_VERSION ||
        history.repository?.toLowerCase() !== expectedRepository.toLowerCase() ||
        !Array.isArray(history.samples) || history.samples.length === 0) {
        throw new Error('Star history is invalid or belongs to another repository.');
    }
    validateRepository(history.repository);
    validateTimestamp(history.repositoryCreatedAt);
    validateTimestamp(history.trackingStartedAt);
    validateTimestamp(history.updatedAt);
    if (Date.parse(history.repositoryCreatedAt) > Date.parse(history.trackingStartedAt) ||
        Date.parse(history.trackingStartedAt) > Date.parse(history.updatedAt)) {
        throw new Error('Star history timestamps are out of order.');
    }
    let previousDate = '';
    for (const sample of history.samples) {
        if (!sample || typeof sample.date !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(sample.date) ||
            !Number.isFinite(Date.parse(`${sample.date}T00:00:00Z`)) ||
            new Date(`${sample.date}T00:00:00Z`).toISOString().slice(0, 10) !== sample.date ||
            sample.date <= previousDate || !Number.isSafeInteger(sample.stars) || sample.stars < 0) {
            throw new Error('Star samples must have unique ordered UTC dates and nonnegative integer totals.');
        }
        previousDate = sample.date;
    }
    if (history.samples[0].date !== history.trackingStartedAt.slice(0, 10) ||
        history.samples.at(-1).date !== history.updatedAt.slice(0, 10)) {
        throw new Error('Star sample dates do not match their observation timestamps.');
    }
    return history;
}

/** Records one real total per UTC day; missed days are never invented.
 * Time O(n), space O(n), preserving the caller's previous history. */
export function recordObservation(previous, repository, createdAt, stars, observedAt) {
    validateRepository(repository);
    validateTimestamp(createdAt);
    validateTimestamp(observedAt);
    if (!Number.isSafeInteger(stars) || stars < 0 || Date.parse(createdAt) > Date.parse(observedAt)) {
        throw new Error('GitHub returned an invalid star count or creation timestamp.');
    }
    if (previous) {
        validateHistory(previous, repository);
        if (Date.parse(previous.repositoryCreatedAt) !== Date.parse(createdAt) ||
            Date.parse(previous.updatedAt) > Date.parse(observedAt)) {
            throw new Error('Repository identity changed or observation time moved backwards.');
        }
    }
    const observation = { date: observedAt.slice(0, 10), stars };
    const samples = previous ? [...previous.samples] : [];
    if (samples.at(-1)?.date === observation.date) {
        if (samples.at(-1).stars === stars) return previous;
        samples[samples.length - 1] = observation;
    } else {
        samples.push(observation);
    }
    return validateHistory({
        schemaVersion: DATA_VERSION,
        repository,
        repositoryCreatedAt: createdAt,
        trackingStartedAt: previous?.trackingStartedAt ?? observedAt,
        updatedAt: observedAt,
        samples,
    });
}

/** Escapes all interpolated text, keeping the generated SVG passive and self-contained. */
function escapeXml(value) {
    return String(value).replace(/[&<>"']/g, character => ({
        '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;',
    })[character]);
}

/** Formats an observation date without depending on the machine's time zone. */
function displayDate(date) {
    return new Intl.DateTimeFormat('en-GB', { day: '2-digit', month: 'short', year: 'numeric', timeZone: 'UTC' })
        .format(new Date(`${date}T00:00:00Z`));
}

/** Gives the y-axis readable headroom, including a true zero-star first observation. */
function axisMaximum(stars) {
    if (stars < 5) return 4;
    const magnitude = 10 ** Math.floor(Math.log10(stars));
    return Math.ceil(stars / (2 * magnitude)) * 2 * magnitude;
}

/** Wraps passive chart markup in an accessible, responsive SVG. */
function chartFrame(repository, description, contents) {
    return `<svg xmlns="http://www.w3.org/2000/svg" width="960" height="336" viewBox="0 0 960 336" role="img" aria-labelledby="title description">
  <title id="title">${escapeXml(repository)} star history</title>
  <desc id="description">${escapeXml(description)}</desc>
  <defs><linearGradient id="area" x1="0" y1="0" x2="0" y2="1"><stop stop-color="#51dfc4" stop-opacity=".23"/><stop offset="1" stop-color="#51dfc4" stop-opacity="0"/></linearGradient></defs>
  <rect x="1" y="1" width="958" height="334" rx="22" fill="#101b21" stroke="#294048"/>
  <g font-family="Segoe UI,Arial,sans-serif">
    <path d="m39 31 3.2 6.5 7.2 1-5.2 5.1 1.2 7.1-6.4-3.4-6.4 3.4 1.2-7.1-5.2-5.1 7.2-1z" fill="#51dfc4"/>
    <text x="61" y="46" fill="#adcbc9" font-size="12" font-weight="700" letter-spacing="1.8">COMMUNITY SUPPORT</text>
    ${contents}
  </g>
</svg>
`;
}

/** Placeholder contains no count or plotted history before the first API observation. */
export function renderPlaceholder(repository) {
    validateRepository(repository);
    return chartFrame(repository, 'Tracking will begin with the first live GitHub observation. No star counts have been recorded yet.', `
    <text x="40" y="111" fill="#eefaf7" font-size="32" font-weight="700">Just launched</text>
    <text x="40" y="152" fill="#a7b9bc" font-size="17">The first live star count will appear here shortly.</text>
    <text x="40" y="185" fill="#a7b9bc" font-size="15">Star the project to follow its progress and help others discover it.</text>
    <path d="M40 249H920" stroke="#294048"/>
    <text x="40" y="291" fill="#91a9ad" font-size="13">Daily UTC observations · ${escapeXml(repository)}</text>`);
}

/** Draws actual observations, with an honest single-point state and no backfill.
 * Time O(n), space O(n), where n is the number of observed days. */
export function renderChart(history) {
    validateHistory(history);
    const first = history.samples[0];
    const latest = history.samples.at(-1);
    const firstTime = Date.parse(`${first.date}T00:00:00Z`);
    const lastTime = Date.parse(`${latest.date}T00:00:00Z`);
    const duration = Math.max(DAY_MS, lastTime - firstTime);
    const maximum = axisMaximum(history.samples.reduce((total, sample) => Math.max(total, sample.stars), 0));
    const coordinate = sample => ({
        x: PLOT.left + (history.samples.length === 1 ? PLOT.width / 2 :
            (Date.parse(`${sample.date}T00:00:00Z`) - firstTime) / duration * PLOT.width),
        y: PLOT.top + PLOT.height * (1 - sample.stars / maximum),
    });
    const points = history.samples.map(coordinate);
    const formatCoordinate = point => `${point.x.toFixed(2)},${point.y.toFixed(2)}`;
    const line = points.map(formatCoordinate).join(' ');
    const latestPoint = points.at(-1);
    const number = new Intl.NumberFormat('en-US');
    const currentCount = `${number.format(latest.stars)} ${latest.stars === 1 ? 'star' : 'stars'}`;
    const firstLabel = history.samples.length === 1 && latest.stars === 0 ? 'Just launched' : 'Star history';
    const subtitle = history.samples.length === 1 ? 'First observation' : 'Latest observation';
    const grid = [0, maximum / 2, maximum].map(value => {
        const y = PLOT.top + PLOT.height * (1 - value / maximum);
        return `<path d="M${PLOT.left} ${y}H${PLOT.left + PLOT.width}" stroke="#294048" stroke-dasharray="3 5"/>
    <text x="48" y="${y + 4}" fill="#91a9ad" font-size="11" text-anchor="end">${number.format(value)}</text>`;
    }).join('\n    ');
    const graph = points.length > 1 ? `<polygon points="${points[0].x},${PLOT.top + PLOT.height} ${line} ${latestPoint.x},${PLOT.top + PLOT.height}" fill="url(#area)"/>
    <polyline points="${line}" fill="none" stroke="#51dfc4" stroke-width="3" stroke-linejoin="round" stroke-linecap="round"/>` : '';
    const firstDateLabel = `<text x="${points.length === 1 ? PLOT.left + PLOT.width / 2 : PLOT.left}" y="278" fill="#91a9ad" font-size="12" text-anchor="${points.length === 1 ? 'middle' : 'start'}">${displayDate(first.date)}</text>`;
    const lastDateLabel = points.length > 1 ? `<text x="${PLOT.left + PLOT.width}" y="278" fill="#91a9ad" font-size="12" text-anchor="end">${displayDate(latest.date)}</text>` : '';
    return chartFrame(history.repository,
        `${currentCount} observed on ${latest.date} UTC. Tracking started ${first.date}; ${history.samples.length} observed days. Missing days are not backfilled.`, `
    <text x="40" y="94" fill="#eefaf7" font-size="27" font-weight="700">${firstLabel}</text>
    <text x="40" y="119" fill="#a7b9bc" font-size="13">Tracking since ${displayDate(first.date)} · ${escapeXml(history.repository)}</text>
    <text x="920" y="84" fill="#51dfc4" font-size="34" font-weight="700" text-anchor="end">${currentCount}</text>
    <text x="920" y="111" fill="#a7b9bc" font-size="12" text-anchor="end">${subtitle} · ${displayDate(latest.date)} UTC</text>
    ${grid}
    ${graph}
    <circle cx="${latestPoint.x.toFixed(2)}" cy="${latestPoint.y.toFixed(2)}" r="5" fill="#51dfc4" stroke="#101b21" stroke-width="2"/>
    ${firstDateLabel}
    ${lastDateLabel}
    <text x="40" y="313" fill="#91a9ad" font-size="12">Daily UTC totals · Stars can rise or fall · Missed days are not backfilled</text>`);
}

/** Makes bounded requests to GitHub only; failures never log tokens or response bodies. */
function githubClient(repository, token, request = fetch) {
    validateRepository(repository);
    if (typeof token !== 'string' || token.trim().length === 0) throw new Error('GITHUB_TOKEN is required for publication.');
    return async (path, method = 'GET', body, allowMissing = false) => {
        const response = await request(`${API_ORIGIN}/repos/${repository}${path}`, {
            method,
            headers: {
                Accept: 'application/vnd.github+json',
                Authorization: `Bearer ${token}`,
                'X-GitHub-Api-Version': API_VERSION,
                'User-Agent': 'SafeBrowse-star-history',
                ...(body ? { 'Content-Type': 'application/json' } : {}),
            },
            body: body ? JSON.stringify(body) : undefined,
            signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
            redirect: 'error',
        });
        if (allowMissing && response.status === 404) return null;
        if (!response.ok) throw new Error(`GitHub ${method} ${path || '/'} failed (${response.status}); history was not force-updated.`);
        return response.json();
    };
}

/** Validates Git object identifiers before using them as request paths. */
function objectId(value) {
    if (typeof value !== 'string' || !/^[a-f0-9]{40,64}$/.test(value)) throw new Error('GitHub returned an invalid Git object identifier.');
    return value;
}

/** Reads only the dedicated data branch and refuses to overwrite unrelated contents. */
async function readPublishedHistory(api, repository) {
    const reference = await api(`/git/ref/heads/${HISTORY_BRANCH}`, 'GET', undefined, true);
    if (!reference) return null;
    const revision = objectId(reference.object?.sha);
    const commit = await api(`/git/commits/${revision}`);
    const treeId = objectId(commit.tree?.sha);
    const tree = await api(`/git/trees/${treeId}?recursive=1`);
    const allowedFiles = new Set([DATA_FILE, CHART_FILE]);
    if (tree.truncated || !Array.isArray(tree.tree) || tree.tree.length !== allowedFiles.size ||
        tree.tree.some(entry => entry.type !== 'blob' || entry.mode !== '100644' || !allowedFiles.delete(entry.path))) {
        throw new Error('The star-history branch contains unexpected files; it will not be overwritten.');
    }
    const dataEntry = tree.tree.find(entry => entry.path === DATA_FILE);
    if (!Number.isSafeInteger(dataEntry.size) || dataEntry.size > MAX_HISTORY_BYTES || dataEntry.size < 1) {
        throw new Error('The star history file exceeds its supported size or is empty.');
    }
    const dataBlob = await api(`/git/blobs/${objectId(dataEntry.sha)}`);
    if (dataBlob.encoding !== 'base64' || typeof dataBlob.content !== 'string' ||
        dataBlob.content.length > Math.ceil(MAX_HISTORY_BYTES / 3) * 4 + MAX_HISTORY_BYTES / 40) {
        throw new Error('GitHub returned unsupported star history contents.');
    }
    const encoded = dataBlob.content.replace(/\s/g, '');
    const decoded = Buffer.from(encoded, 'base64');
    if (decoded.length > MAX_HISTORY_BYTES || decoded.toString('base64') !== encoded) {
        throw new Error('Star history encoding is invalid or too large.');
    }
    const history = validateHistory(JSON.parse(decoded.toString('utf8')), repository);
    return { revision, treeId, history };
}

/** Publishes only data/SVG; a concurrent update fails rather than losing its history.
 * Time O(n), space O(n) for n daily observations; a fixed number of API requests. */
export async function publishHistory({ repository, token, request = fetch, now = new Date() }) {
    const api = githubClient(repository, token, request);
    const metadata = await api('');
    if (metadata.full_name?.toLowerCase() !== repository.toLowerCase() || metadata.private !== false) {
        throw new Error('Star tracking requires the intended public repository.');
    }
    const previous = await readPublishedHistory(api, repository);
    const history = recordObservation(previous?.history, metadata.full_name, metadata.created_at,
        metadata.stargazers_count, now.toISOString());
    if (history === previous?.history) return { changed: false, stars: metadata.stargazers_count };
    const data = `${JSON.stringify(history, null, 2)}\n`;
    if (Buffer.byteLength(data) > MAX_HISTORY_BYTES) throw new Error('Star history exceeds the supported file size.');
    const tree = await api('/git/trees', 'POST', {
        ...(previous ? { base_tree: previous.treeId } : {}),
        tree: [
            { path: DATA_FILE, mode: '100644', type: 'blob', content: data },
            { path: CHART_FILE, mode: '100644', type: 'blob', content: renderChart(history) },
        ],
    });
    const commit = await api('/git/commits', 'POST', {
        message: `Update star history for ${history.updatedAt.slice(0, 10)}`,
        tree: objectId(tree.sha),
        parents: previous ? [previous.revision] : [],
    });
    const newRevision = objectId(commit.sha);
    if (previous) {
        await api(`/git/refs/heads/${HISTORY_BRANCH}`, 'PATCH', { sha: newRevision, force: false });
    } else {
        await api('/git/refs', 'POST', { ref: `refs/heads/${HISTORY_BRANCH}`, sha: newRevision });
    }
    return { changed: true, stars: metadata.stargazers_count };
}

/** Keeps local rendering offline; only the explicit --publish command uses a token. */
async function main(argumentsList) {
    if (argumentsList.length === 1 && argumentsList[0] === '--publish') {
        const result = await publishHistory({ repository: process.env.GITHUB_REPOSITORY, token: process.env.GITHUB_TOKEN });
        console.log(result.changed ? 'Published the latest observed star total.' : 'Today\'s observed total is unchanged.');
        return;
    }
    if (argumentsList.length === 3 && argumentsList[0] === '--render') {
        const contents = await readFile(argumentsList[1]);
        if (contents.length > MAX_HISTORY_BYTES) throw new Error('Star history exceeds the supported file size.');
        await writeFile(argumentsList[2], renderChart(JSON.parse(contents.toString('utf8'))), { flag: 'wx' });
        return;
    }
    if (argumentsList.length === 3 && argumentsList[0] === '--placeholder') {
        await writeFile(argumentsList[2], renderPlaceholder(argumentsList[1]), { flag: 'wx' });
        return;
    }
    if (argumentsList.length === 0 || (argumentsList.length === 1 && argumentsList[0] === '--help')) {
        console.log('Usage: node scripts/update-star-history.mjs --publish\n       node scripts/update-star-history.mjs --render stars.json output.svg\n       node scripts/update-star-history.mjs --placeholder OWNER/REPOSITORY output.svg');
        return;
    }
    throw new Error('Unrecognized arguments. Use --help for supported commands.');
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
    main(process.argv.slice(2)).catch(error => {
        console.error(`Star history: ${error.message}`);
        process.exitCode = 1;
    });
}
