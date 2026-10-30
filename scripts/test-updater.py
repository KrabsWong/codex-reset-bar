#!/usr/bin/env python3
"""Exercise the production Sparkle bridge and its persisted preference in an isolated bundle."""
import argparse
from pathlib import Path
import plistlib
import subprocess
import tempfile
import uuid

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--sdk', type=Path, default=root / 'build/sparkle/sdk')
args = parser.parse_args()
framework = args.sdk.resolve() / 'Sparkle.xcframework/macos-arm64_x86_64'
with tempfile.TemporaryDirectory(prefix='updater-smoke-') as directory:
    work = Path(directory)
    app = work / 'UpdaterSmoke.app'
    binary = app / 'Contents/MacOS/UpdaterSmoke'
    binary.parent.mkdir(parents=True)
    (app / 'Contents/Frameworks').mkdir()
    identifier = 'com.krabs.codex-reset.updater-test.' + uuid.uuid4().hex
    info = dict(CFBundleIdentifier=identifier, CFBundleExecutable='UpdaterSmoke',
                CFBundleName='Updater Smoke', CFBundlePackageType='APPL', CFBundleVersion='1.0.0',
                SUFeedURL='https://example.invalid/appcast.xml',
                SUPublicEDKey='AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=',
                SUEnableAutomaticChecks=True, SUAutomaticallyUpdate=False, SUAllowsAutomaticUpdates=False,
                SURequireSignedFeed=True, SUVerifyUpdateBeforeExtraction=True)
    (app / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
    source = work / 'main.m'
    source.write_text('''#import <AppKit/AppKit.h>
#include <stdbool.h>
#include <assert.h>
extern bool codex_updater_start(void), codex_updater_available(void), codex_updater_automatic(void);
extern void codex_updater_set_automatic(bool);
int main(int argc, const char **argv) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        if (strcmp(argv[1], "cleanup") == 0) {
            [NSUserDefaults.standardUserDefaults removePersistentDomainForName:NSBundle.mainBundle.bundleIdentifier];
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
    def run(*command):
        subprocess.run([str(value) for value in command], check=True)
    run('ditto', framework / 'Sparkle.framework', app / 'Contents/Frameworks/Sparkle.framework')
    run('clang', '-fobjc-arc', '-mmacosx-version-min=14.0', '-F', framework,
        '-framework', 'AppKit', '-framework', 'Sparkle', '-Wl,-rpath,@executable_path/../Frameworks',
        source, root / 'native/updater.m', '-o', binary)
    run('codesign', '--force', '--deep', '--sign', '-', app)
    try:
        run(binary, '1', '0')
        run(binary, '0', '1')
        run(binary, '1', '1')
        print('PASS: production updater starts; automatic-check setting survives two relaunches')
    finally:
        run(binary, 'cleanup')
