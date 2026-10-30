#!/usr/bin/env python3
"""Build and run a local, ad-hoc signed A/B experiment. No release publishing."""
import argparse
import functools
import hashlib
import http.server
import json
import os
from pathlib import Path
import plistlib
import re
import signal
import subprocess
import tempfile
import threading
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--sdk', type=Path, required=True, help='Extracted Sparkle distribution with bin/ and Sparkle.xcframework/')
parser.add_argument('--bad-signature', action='store_true', help='Verify rejection of an incorrectly signed archive')
parser.add_argument('--distribution', action='store_true', help='Serve A for a browser download, then observe its update in /Applications')
args = parser.parse_args()
if args.distribution and args.bad_signature:
    parser.error('--distribution and --bad-signature are separate experiments')
sdk = args.sdk.resolve()
framework_dir = sdk / 'Sparkle.xcframework/macos-arm64_x86_64'
(ROOT / 'build/sparkle-poc').mkdir(parents=True, exist_ok=True)
work = Path(tempfile.mkdtemp(prefix='run-', dir=ROOT / 'build/sparkle-poc'))
print(f'Evidence: {work}', flush=True)


def run(*cmd, **kwargs):
    return subprocess.run([str(c) for c in cmd], check=True, **kwargs)


# Separate target directory keeps the normal release executable untouched.
env = dict(os.environ, SPARKLE_FRAMEWORK_DIR=str(framework_dir))
run('cargo', 'build', '--release', '--features', 'sparkle-poc', '--target-dir', ROOT / 'target/sparkle-poc', cwd=ROOT, env=env)
serve = work / 'feed'
serve.mkdir()
class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, fmt, *values):
        with (work / 'http.log').open('a') as log:
            log.write((fmt % values) + '\n')

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(serve)))
threading.Thread(target=server.serve_forever, daemon=True).start()
origin = f'http://127.0.0.1:{server.server_port}'
key = work / 'test-key'
# Ephemeral test seed; never reads or writes the user's signing keychain.
swift = work / 'key.swift'
swift.write_text('''import Foundation
import CryptoKit
let key = Curve25519.Signing.PrivateKey()
try key.rawRepresentation.base64EncodedString().write(toFile: CommandLine.arguments[1], atomically: true, encoding: .utf8)
print(key.publicKey.rawRepresentation.base64EncodedString())
''')
public_key = run('swift', '-module-cache-path', work / 'swift-cache', swift, key, capture_output=True, text=True).stdout.strip()
key.chmod(0o600)
identifier = 'com.krabs.codex-reset.sparkle-poc.' + uuid.uuid4().hex
log_path = work / 'events.log'
data = work / 'data'
data.mkdir()
sentinel = data / 'settings.json'
sentinel.write_text('{"language":"en"}')
original_settings = sentinel.read_bytes()
apps = []
try:
    for version, parent in [('1', work / 'installed'), ('2', work / 'candidate')]:
        app = parent / (f'Codex Resets Gatekeeper {identifier[-8:]}.app' if args.distribution else 'Codex Resets Sparkle PoC.app')
        contents = app / 'Contents'
        (contents / 'MacOS').mkdir(parents=True)
        (contents / 'Frameworks').mkdir()
        run('ditto', ROOT / 'target/sparkle-poc/release/codex-reset', contents / 'MacOS/CodexResetsPoC')
        run('ditto', framework_dir / 'Sparkle.framework', contents / 'Frameworks/Sparkle.framework')
        info = plistlib.loads((ROOT / 'resources/Info.plist').read_bytes())
        info.update(CFBundleIdentifier=identifier, CFBundleExecutable='CodexResetsPoC',
                    CFBundleName='Codex Resets Sparkle PoC', CFBundleDisplayName='Codex Resets Sparkle PoC',
                    CFBundleVersion=version, CFBundleShortVersionString='0.0.' + version,
                    SUFeedURL=origin + '/appcast.xml', SUPublicEDKey=public_key,
                    SUEnableAutomaticChecks=False, SUAutomaticallyUpdate=False,
                    SUVerifyUpdateBeforeExtraction=True, PoCLogPath=str(log_path), PoCDataPath=str(data),
                    NSAppTransportSecurity={'NSAllowsLocalNetworking': True})
        info.pop('CFBundleIconFile', None)
        (contents / 'Info.plist').write_bytes(plistlib.dumps(info))
        # Re-sign ALL nested test code ad-hoc: no Apple signing identity involved.
        run('codesign', '--force', '--deep', '--sign', '-', app)
        run('codesign', '--verify', '--deep', '--strict', app)
        details = run('codesign', '-dvv', app, capture_output=True, text=True).stderr
        (work / f'signing-{version}.txt').write_text(details)
        assert 'Signature=adhoc' in details and 'TeamIdentifier=not set' in details
        apps.append(app)
    archive = serve / 'B.zip'
    run('ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', apps[1], archive)
    attributes = run(sdk / 'bin/sign_update', '--ed-key-file', key, archive, capture_output=True, text=True).stdout.strip()
    if args.bad_signature:
        attributes = re.sub(r'edSignature="(.)', lambda m: 'edSignature="' + ('B' if m[1] == 'A' else 'A'), attributes)
    (serve / 'appcast.xml').write_text(f'''<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel>
<title>Local Sparkle PoC</title><item><title>B</title><sparkle:version>2</sparkle:version>
<sparkle:shortVersionString>0.0.2</sparkle:shortVersionString><sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
<enclosure url="{origin}/B.zip" {attributes} type="application/octet-stream"/>
</item></channel></rss>''')
    key.unlink()
    before = hashlib.sha256((apps[0] / 'Contents/Info.plist').read_bytes()).hexdigest()
    if args.distribution:
        run('ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', apps[0], serve / 'A.zip')
        (serve / 'index.html').write_text('<!doctype html><title>Sparkle Gatekeeper test</title><h1>Ad-hoc signed test app A</h1><a href="A.zip" download="Codex-Resets-Gatekeeper-A.zip">Download test version A</a>')
        apps[0] = Path('/Applications') / apps[0].name
        (work / 'distribution.json').write_text(json.dumps(dict(url=origin, app=str(apps[0]), work=str(work)), indent=2))
        print(f'BROWSER DOWNLOAD: {origin}/\nINSTALL TARGET: {apps[0]}', flush=True)
    else:
        run('open', '-n', apps[0])
    deadline = time.monotonic() + (3600 if args.distribution else 100)
    events = ''
    while time.monotonic() < deadline:
        events = log_path.read_text() if log_path.exists() else ''
        if 'app-ready version=2' in events or ('ERROR ' in events and not args.distribution):
            break
        time.sleep(1)
    print(events, flush=True)
    installed = plistlib.loads((apps[0] / 'Contents/Info.plist').read_bytes())
    if args.bad_signature:
        assert 'Code=4005' in events and 'EdDSA signature does not match' in events, 'Expected signature rejection'
        assert 'app-ready version=2' not in events
        os.kill(int(re.search(r'app-ready version=1 pid=(\d+)', events)[1]), 0)
        run('codesign', '--verify', '--deep', '--strict', apps[0])
        assert hashlib.sha256((apps[0] / 'Contents/Info.plist').read_bytes()).hexdigest() == before
    else:
        assert 'ready-to-relaunch' in events and 'app-ready version=2' in events, 'A -> B relaunch not observed'
        assert installed['CFBundleVersion'] == '2'
        pids = re.findall(r'app-ready version=([12]) pid=(\d+)', events)
        assert len(pids) >= 2 and pids[0][0] == '1' and pids[-1][0] == '2'
        assert pids[0][1] != pids[-1][1]
        os.kill(int(pids[-1][1]), 0)
        run('codesign', '--verify', '--deep', '--strict', apps[0])
    assert sentinel.read_bytes() == original_settings, 'Settings changed'
    report = dict(result='PASS', bad_signature=args.bad_signature, installed_version=installed['CFBundleVersion'],
                  bundle_id=identifier, sdk=str(sdk), platform=run('sw_vers', capture_output=True, text=True).stdout,
                  events=events, scope=('Browser download and /Applications; inspect quarantine and Gatekeeper evidence separately' if args.distribution else 'Local HTTP, user-writable directory, no quarantine; not a Gatekeeper distribution test'))
    (work / 'result.json').write_text(json.dumps(report, indent=2))
    print('PASS', flush=True)
finally:
    key.unlink(missing_ok=True)
    server.shutdown()
    # Stop only this run's app, using the unique bundle path rather than a shared process name.
    processes = run('ps', '-axo', 'pid=,command=', capture_output=True, text=True).stdout
    for line in processes.splitlines():
        if apps and str(apps[0]) in line and '/Contents/MacOS/CodexResetsPoC' in line:
            os.kill(int(line.strip().split(None, 1)[0]), signal.SIGTERM)
