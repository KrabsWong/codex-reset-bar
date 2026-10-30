#!/usr/bin/env python3
"""Exercise production updater preferences and gentle reminders in an isolated bundle."""
import argparse
import functools
import http.server
from pathlib import Path
import plistlib
import subprocess
import tempfile
import threading
import uuid

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--sdk', type=Path, default=root / 'build/sparkle/sdk')
args = parser.parse_args()
framework = args.sdk.resolve() / 'Sparkle.xcframework/macos-arm64_x86_64'
with tempfile.TemporaryDirectory(prefix='updater-smoke-') as directory:
    work = Path(directory)
    requests = []
    class Handler(http.server.SimpleHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            super().do_GET()

        def log_message(self, *_):
            pass

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(work)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    origin = f'http://127.0.0.1:{server.server_port}'
    def run(*command, **kwargs):
        return subprocess.run([str(value) for value in command], check=True, **kwargs)

    # Ephemeral fixture key; never touches the production keychain or GitHub key.
    key = work / 'fixture-key'
    swift = '''import Foundation
import CryptoKit
let key = Curve25519.Signing.PrivateKey()
try key.rawRepresentation.base64EncodedString().write(toFile: CommandLine.arguments[1], atomically: true, encoding: .utf8)
print(key.publicKey.rawRepresentation.base64EncodedString())
'''
    key_source = work / 'key.swift'
    key_source.write_text(swift)
    public_key = run('swift', '-module-cache-path', work / 'swift-cache', key_source, key,
                     capture_output=True, text=True).stdout.strip()
    key.chmod(0o600)
    archive = work / 'update.zip'
    archive.write_bytes(b'Fixture only: the test must never download or install this file.')
    attributes = run(args.sdk.resolve() / 'bin/sign_update', '--ed-key-file', key, archive,
                     capture_output=True, text=True).stdout.strip()
    feed = work / 'appcast.xml'
    feed.write_text(f'''<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel>
<title>Gentle reminder test</title><item><title>Test 1.0.1</title>
<sparkle:version>1.0.1</sparkle:version><sparkle:shortVersionString>1.0.1</sparkle:shortVersionString>
<enclosure url="{origin}/update.zip" {attributes} type="application/octet-stream"/>
</item></channel></rss>''')
    run(args.sdk.resolve() / 'bin/sign_update', '--ed-key-file', key, feed, capture_output=True)
    key.unlink()
    app = work / 'UpdaterSmoke.app'
    binary = app / 'Contents/MacOS/UpdaterSmoke'
    binary.parent.mkdir(parents=True)
    (app / 'Contents/Frameworks').mkdir()
    identifier = 'com.krabs.codex-reset.updater-test.' + uuid.uuid4().hex
    info = dict(CFBundleIdentifier=identifier, CFBundleExecutable='UpdaterSmoke',
                CFBundleName='Updater Smoke', CFBundlePackageType='APPL', CFBundleVersion='1.0.0',
                SUFeedURL=origin + '/appcast.xml', SUPublicEDKey=public_key,
                SUEnableAutomaticChecks=True, SUAutomaticallyUpdate=False, SUAllowsAutomaticUpdates=False,
                SURequireSignedFeed=True, SUVerifyUpdateBeforeExtraction=True,
                NSAppTransportSecurity={'NSAllowsLocalNetworking': True})
    (app / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
    source = work / 'main.m'
    source.write_text('''#import <AppKit/AppKit.h>
#include <stdbool.h>
#include <assert.h>
extern bool codex_updater_start(void), codex_updater_available(void), codex_updater_automatic(void);
extern void codex_updater_set_automatic(bool);
extern bool codex_updater_can_check(void);
extern const char *codex_updater_version(void);
extern void codex_updater_set_handler(void (*handler)(const char *));
extern void codex_updater_check(void);
static bool receivedVersion, clearedVersion;
static void changed(const char *version) {
    if (version) {
        assert(strcmp(version, "1.0.1") == 0);
        receivedVersion = true;
    } else if (receivedVersion) {
        clearedVersion = true;
    }
}
static void pump(double seconds) {
    NSDate *end = [NSDate dateWithTimeIntervalSinceNow:seconds];
    while (end.timeIntervalSinceNow > 0) {
        [NSRunLoop.mainRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
    }
}
static bool hasVisibleWindow(void) {
    for (NSWindow *window in NSApp.windows) if (window.visible) return true;
    return false;
}
int main(int argc, const char **argv) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        if (strcmp(argv[1], "cleanup") == 0) {
            [NSUserDefaults.standardUserDefaults removePersistentDomainForName:NSBundle.mainBundle.bundleIdentifier];
        } else if (strcmp(argv[1], "disabled") == 0) {
            [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
            [NSApp finishLaunching];
            assert(codex_updater_start());
            assert(!codex_updater_automatic());
            pump(1.0);
            assert(codex_updater_version() == NULL);
            assert(!hasVisibleWindow());
        } else if (strcmp(argv[1], "gentle") == 0 || strcmp(argv[1], "manual") == 0) {
            bool automatic = strcmp(argv[1], "gentle") == 0;
            [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
            [NSApp finishLaunching];
            [NSUserDefaults.standardUserDefaults removeObjectForKey:@"SULastCheckTime"];
            codex_updater_set_handler(changed);
            assert(codex_updater_start());
            assert(codex_updater_automatic() == automatic);
            if (!automatic) codex_updater_check();
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:20];
            while (!receivedVersion && deadline.timeIntervalSinceNow > 0) {
                pump(0.05);
                if (automatic) assert(!hasVisibleWindow());
            }
            assert(receivedVersion);
            assert(strcmp(codex_updater_version(), "1.0.1") == 0);
            assert(codex_updater_can_check()); // Pending sessions must remain actionable.
            if (automatic) {
                pump(0.5);
                assert(!hasVisibleWindow());
                codex_updater_check(); // Explicit user action brings the pending update forward.
            }
            deadline = [NSDate dateWithTimeIntervalSinceNow:5];
            while (!hasVisibleWindow() && deadline.timeIntervalSinceNow > 0) pump(0.05);
            assert(hasVisibleWindow());
            for (NSWindow *window in NSApp.windows) if (window.visible) [window performClose:nil];
            deadline = [NSDate dateWithTimeIntervalSinceNow:5];
            while (!clearedVersion && deadline.timeIntervalSinceNow > 0) pump(0.05);
            assert(clearedVersion);
            assert(codex_updater_version() == NULL);
            assert(codex_updater_can_check());
            codex_updater_set_handler(NULL);
        } else {
            assert(codex_updater_start());
            assert(codex_updater_start()); // Starting twice must preserve the same controller/settings.
            assert(codex_updater_available());
            assert(codex_updater_automatic() == (argv[1][0] == '1'));
            codex_updater_set_automatic(argv[2][0] == '1');
            assert(codex_updater_automatic() == (argv[2][0] == '1'));
        }
        [NSUserDefaults.standardUserDefaults synchronize];
    }
    return 0;
}
''')
    run('ditto', framework / 'Sparkle.framework', app / 'Contents/Frameworks/Sparkle.framework')
    run('clang', '-fobjc-arc', '-mmacosx-version-min=14.0', '-F', framework,
        '-framework', 'AppKit', '-framework', 'Sparkle', '-Wl,-rpath,@executable_path/../Frameworks',
        source, root / 'native/updater.m', '-o', binary)
    run('codesign', '--force', '--deep', '--sign', '-', app)
    try:
        run(binary, '1', '0')
        run(binary, 'disabled', timeout=10)
        assert not requests, 'Disabled automatic checks unexpectedly requested the feed'
        run(binary, 'manual', timeout=40)
        assert '/appcast.xml' in requests, 'Manual check did not request the signed feed'
        assert '/update.zip' not in requests, 'Manual check unexpectedly downloaded an update'
        requests.clear()
        print('PASS: manual checking still shows Sparkle with automatic checks disabled; dismissal clears reminder')
        run(binary, '0', '1')
        run(binary, '1', '1')
        print('PASS: production updater starts; automatic-check setting survives two relaunches')
        run(binary, 'gentle', timeout=40)
        assert '/appcast.xml' in requests, 'Automatic discovery did not request the signed feed'
        assert '/update.zip' not in requests, 'Reminder test unexpectedly downloaded an update'
        print('PASS: signed automatic discovery stays silent; manual action shows Sparkle; dismissal clears reminder')
    finally:
        run(binary, 'cleanup')
        server.shutdown()
