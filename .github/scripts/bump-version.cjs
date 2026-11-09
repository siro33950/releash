const { execFileSync } = require('node:child_process');
const { readFileSync, writeFileSync } = require('node:fs');

execFileSync('./nx', ['release', 'version', process.argv[2]], { stdio: 'inherit' });
const version = JSON.parse(readFileSync('clients/desktop/package.json')).version;
const tauri = JSON.parse(readFileSync('clients/desktop/tauri.conf.json'));
tauri.version = version;
writeFileSync('clients/desktop/tauri.conf.json', `${JSON.stringify(tauri, null, '\t')}\n`);
writeFileSync('Cargo.toml', readFileSync('Cargo.toml', 'utf8').replace(/^version = ".*"/m, `version = "${version}"`));
writeFileSync('Cargo.lock', readFileSync('Cargo.lock', 'utf8').replace(/(name = "(?:releashd|releash|releash-desktop)"\n)version = ".*"/g, `$1version = "${version}"`));
writeFileSync('clients/macos/project.yml', readFileSync('clients/macos/project.yml', 'utf8').replace(/^    MARKETING_VERSION:.*$/m, `    MARKETING_VERSION: '${version}'`));
