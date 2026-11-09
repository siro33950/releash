const { execFileSync } = require('node:child_process');
const { readFileSync, writeFileSync, mkdirSync, statSync } = require('node:fs');
const { join } = require('node:path');
const { createPublicKey, verify } = require('node:crypto');
const marker = /<!-- sparkle:([^\n]+) -->/g;

function metadata(release) {
  const match = [...(release.body || '').matchAll(marker)].at(-1);
  if (!match) throw new Error(`Missing Sparkle signature: ${release.tag_name}`);
  const item = JSON.parse(match[1]);
  if (!/^\d+\.\d+\.\d+$/.test(item.version) || !/^[A-Za-z0-9+/]{86}==$/.test(item.signature)
      || !Number.isSafeInteger(item.length) || item.length <= 0) throw new Error('Invalid Sparkle metadata');
  if (release.tag_name.match(/^v(\d+\.\d+\.\d+)(?:-nightly\.\d{8}\.[1-9]\d*)?$/)?.[1] !== item.version) throw new Error('Release version mismatch');
  const asset = release.assets.find(asset => asset.name === `Releash_${item.version}_universal.zip` && asset.size === item.length);
  if (!asset) throw new Error('Missing Swift asset');
  return { ...item, url: asset.browser_download_url, nightly: release.prerelease };
}

function selectReleases(releases) {
  const candidates = releases.filter(release => !release.draft && (release.body || '').includes('<!-- sparkle:') && /^v\d+\.\d+\.\d+(?:-nightly\.\d{8}\.\d+)?$/.test(release.tag_name))
    .sort((a, b) => b.tag_name.localeCompare(a.tag_name, undefined, { numeric: true }));
  const stable = candidates.find(release => !release.prerelease);
  const nightly = candidates.find(release => release.prerelease);
  return [stable, nightly].filter(Boolean);
}

function appcast(releases) {
  const selected = selectReleases(releases).map(metadata);
  const entries = selected.filter(item => !item.nightly || !selected.some(other => !other.nightly && other.version === item.version));
  const xml = value => String(value).replaceAll('&', '&amp;').replaceAll('"', '&quot;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
  return `<?xml version="1.0" encoding="utf-8"?>\n<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><title>Releash</title>${entries.map(item => `
<item><title>${item.version}</title><sparkle:version>${item.version}</sparkle:version><sparkle:shortVersionString>${item.version}</sparkle:shortVersionString>${item.nightly ? '<sparkle:channel>nightly</sparkle:channel>' : ''}<enclosure url="${xml(item.url)}" sparkle:edSignature="${item.signature}" length="${item.length}" type="application/octet-stream"/></item>`).join('')}\n</channel></rss>\n`;
}

function verifyArchive(file, signature, publicKey) {
  const raw = Buffer.from(publicKey, 'base64');
  if (raw.length !== 32) throw new Error('Invalid Sparkle public key');
  const key = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), raw]), format: 'der', type: 'spki' });
  if (!verify(null, readFileSync(file), key, Buffer.from(signature, 'base64'))) throw new Error('Sparkle signature does not match application public key');
}

function main(mode, run = (...args) => execFileSync(args[0], args.slice(1), { encoding: 'utf8' }).trim()) {
  if (mode === 'verify') {
    const item = JSON.parse(readFileSync(process.argv[3]));
    verifyArchive(process.argv[4], item.signature, process.env.SPARKLE_PUBLIC_ED_KEY);
    return;
  }
  const repo = process.env.GITHUB_REPOSITORY;
  const tag = process.env.RELEASE_TAG;
  const api = path => JSON.parse(run('gh', 'api', path));
  if (mode === 'upload') {
    const release = api(`repos/${repo}/releases/tags/${tag}`);
    if (!release.draft) { metadata(release); return; }
    const item = JSON.parse(readFileSync(join(process.env.ASSETS, 'sparkle.json')));
    const file = join(process.env.ASSETS, `Releash_${item.version}_universal.zip`);
    if (statSync(file).size !== item.length) throw new Error('Archive length mismatch');
    run('gh', 'release', 'upload', tag, file, '--clobber', '--repo', repo);
    const refreshed = api(`repos/${repo}/releases/tags/${tag}`);
    const body = `${(release.body || '').replace(marker, '').trim()}\n\n<!-- sparkle:${JSON.stringify(item)} -->`;
    metadata({ ...refreshed, body });
    writeFileSync(join(process.env.ASSETS, 'body.txt'), body);
    run('gh', 'release', 'edit', tag, '--notes-file', join(process.env.ASSETS, 'body.txt'), '--repo', repo);
    return;
  }
  let target;
  let candidates = [];
  let latest = process.env.NIGHTLY !== 'true';
  for (let page = 1; ; page++) {
    const releases = api(`repos/${repo}/releases?per_page=30&page=${page}`);
    target ??= releases.find(release => release.tag_name === tag);
    if (releases.some(release => !release.draft && !release.prerelease && /^v\d+\.\d+\.\d+$/.test(release.tag_name)
        && release.tag_name.localeCompare(tag, undefined, { numeric: true }) > 0)) latest = false;
    candidates = selectReleases([...candidates, ...releases.map(release =>
      release.tag_name === tag ? { ...release, draft: false, prerelease: process.env.NIGHTLY === 'true' } : release)]);
    if (releases.length < 30) break;
  }
  if (!target) throw new Error('Release not found');
  metadata(target);
  const planned = appcast(candidates);
  mkdirSync('public', { recursive: true });
  writeFileSync('public/appcast.xml', planned);
  run('gh', 'release', 'edit', tag, '--draft=false', `--prerelease=${process.env.NIGHTLY === 'true'}`, `--latest=${latest}`, '--repo', repo);
  // The feed is deployed only after the Release and its asset are public.
  const xml = readFileSync('public/appcast.xml', 'utf8');
  for (const url of [...xml.matchAll(/url="([^"]+)"/g)].map(match => match[1].replaceAll('&amp;', '&'))) {
    run('curl', '--fail', '--location', '--head', '--retry', '5', '--retry-all-errors', url);
  }
}

module.exports = { metadata, appcast, verifyArchive, main };
if (require.main === module) main(process.argv[2]);
