const { readFileSync, writeFileSync } = require('node:fs');

function nextVersion(current, bump) {
  const [major, minor, patch] = current.split('.').map(Number);
  if (bump === 'major') return `${major + 1}.0.0`;
  if (bump === 'minor') return `${major}.${minor + 1}.0`;
  if (bump === 'patch') return `${major}.${minor}.${patch + 1}`;
  if (/^\d+\.\d+\.\d+$/.test(bump)) return bump;
  throw new Error('Expected major, minor, patch or X.Y.Z');
}

const cargo = readFileSync('Cargo.toml', 'utf8');
const version = nextVersion(cargo.match(/^version = "(.*)"/m)[1], process.argv[2]);
writeFileSync('Cargo.toml', cargo.replace(/^version = ".*"/m, `version = "${version}"`));
writeFileSync('Cargo.lock', readFileSync('Cargo.lock', 'utf8').replace(/(name = "(?:releashd|releash)"\n)version = ".*"/g, `$1version = "${version}"`));
writeFileSync('clients/macos/project.yml', readFileSync('clients/macos/project.yml', 'utf8').replace(/^    MARKETING_VERSION:.*$/m, `    MARKETING_VERSION: '${version}'`));
