#!/usr/bin/env python3
"""Create signed Sparkle release assets, or publish a complete draft release."""
import argparse
import base64
import json
import html
import os
from pathlib import Path
import plistlib
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
REPO = 'KrabsWong/codex-reset-bar'
FEED_URL = f'https://github.com/{REPO}/releases/latest/download/appcast.xml'
SPARKLE = 'http://www.andymatuschak.org/xml-namespaces/sparkle'


def run(*args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def version_tuple(tag):
    if not re.fullmatch(r'v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)', tag):
        raise ValueError('Release tag must be a stable vMAJOR.MINOR.PATCH version')
    return tuple(int(part) for part in tag[1:].split('.'))


def git(root, *args):
    return subprocess.check_output(['git', *args], cwd=root, text=True).strip()


def validate_tag(root, tag, main_ref='refs/remotes/origin/main'):
    version_tuple(tag)
    commit = git(root, 'rev-parse', '--verify', f'refs/tags/{tag}^{{commit}}')
    if git(root, 'rev-parse', 'HEAD') != commit:
        raise ValueError('Release tag must point at the checked-out commit')
    ancestry = subprocess.run(['git', 'merge-base', '--is-ancestor', commit, main_ref], cwd=root, capture_output=True)
    if ancestry.returncode != 0:
        raise ValueError('Release tag must point to a commit already merged into origin/main')
    return commit


def validate_bundle(info, tag, public_key):
    version_tuple(tag)
    if not public_key or len(base64.b64decode(public_key, validate=True)) != 32:
        raise ValueError('SPARKLE_PUBLIC_ED_KEY must be a base64 32-byte public key')
    expected = dict(CFBundleIdentifier='com.krabs.codex-reset', CFBundleVersion=tag[1:],
                    CFBundleShortVersionString=tag[1:], SUFeedURL=FEED_URL,
                    SUPublicEDKey=public_key, SUVerifyUpdateBeforeExtraction=True,
                    SURequireSignedFeed=True)
    if any(info.get(key) != value for key, value in expected.items()):
        raise ValueError('Bundle version, updater configuration, or public key does not match release')


def asset_paths(tag):
    return ROOT / f'dist/Codex-Resets-{tag[1:]}-macOS-arm64.zip', ROOT / 'dist/appcast.xml'


def prepare(tag, sdk):
    app = ROOT / 'dist/Codex Resets.app'
    info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    public_key = os.environ.get('SPARKLE_PUBLIC_ED_KEY', '').strip()
    validate_bundle(info, tag, public_key)
    private_key = os.environ.get('SPARKLE_PRIVATE_ED_KEY', '').strip()
    if not private_key or len(base64.b64decode(private_key, validate=True)) != 32:
        raise ValueError('SPARKLE_PRIVATE_ED_KEY must be a base64 32-byte Ed25519 seed')
    # Only the public key is emitted. Never put the seed in command arguments or files.
    swift = '''import Foundation
import CryptoKit
let data = FileHandle.standardInput.readDataToEndOfFile()
let seed = Data(base64Encoded: String(data: data, encoding: .utf8)!.trimmingCharacters(in: .whitespacesAndNewlines))!
let key = try Curve25519.Signing.PrivateKey(rawRepresentation: seed)
print(key.publicKey.rawRepresentation.base64EncodedString())
'''
    actual_key = run('swift', '-module-cache-path', ROOT / 'build/swift-cache', '-e', swift,
                     input=private_key, capture_output=True, text=True).stdout.strip()
    if actual_key != public_key:
        raise ValueError('Release signing seed does not match the public key embedded in the app')
    run('codesign', '--verify', '--deep', '--strict', app)
    details = run('codesign', '-dvv', app, capture_output=True, text=True).stderr
    if 'Signature=adhoc' not in details or 'TeamIdentifier=not set' not in details:
        raise ValueError('Expected an ad-hoc signed application with no Team ID')
    binary = app / 'Contents/MacOS' / info['CFBundleExecutable']
    if run('lipo', '-archs', binary, capture_output=True, text=True).stdout.strip() != 'arm64':
        raise ValueError('Release executable must be arm64')
    archive, feed = asset_paths(tag)
    archive.unlink(missing_ok=True)
    run('ditto', '-c', '-k', '--sequesterRsrc', '--keepParent', app, archive)
    signer = sdk / 'bin/sign_update'
    signature = run(signer, '--ed-key-file', '-', '-p', archive,
                    input=private_key, capture_output=True, text=True).stdout.strip()
    if len(base64.b64decode(signature, validate=True)) != 64:
        raise ValueError('Sparkle signer did not return an Ed25519 signature')
    ET.register_namespace('sparkle', SPARKLE)
    root = ET.Element('rss', version='2.0')
    channel = ET.SubElement(root, 'channel')
    ET.SubElement(channel, 'title').text = 'Codex Resets updates'
    item = ET.SubElement(channel, 'item')
    ET.SubElement(item, 'title').text = f'Codex Resets {tag[1:]}'
    ET.SubElement(item, 'link').text = f'https://github.com/{REPO}/releases/tag/{tag}'
    ET.SubElement(item, f'{{{SPARKLE}}}version').text = tag[1:]
    ET.SubElement(item, f'{{{SPARKLE}}}shortVersionString').text = tag[1:]
    ET.SubElement(item, f'{{{SPARKLE}}}minimumSystemVersion').text = info['LSMinimumSystemVersion']
    ET.SubElement(item, f'{{{SPARKLE}}}hardwareRequirements').text = 'arm64'
    ET.SubElement(item, 'enclosure', url=f'https://github.com/{REPO}/releases/download/{tag}/{archive.name}',
                  length=str(archive.stat().st_size), type='application/octet-stream',
                  attrib={f'{{{SPARKLE}}}edSignature': signature})
    ET.ElementTree(root).write(feed, encoding='utf-8', xml_declaration=True)
    run(signer, '--ed-key-file', '-', feed, input=private_key, capture_output=True, text=True)
    print(f'Prepared {archive.name} and signed appcast.xml')


def github_release(endpoint):
    response = subprocess.run(['gh', 'api', f'repos/{REPO}/releases/{endpoint}'], capture_output=True, text=True)
    if response.returncode:
        if '(HTTP 404)' in response.stderr:
            return None
        raise RuntimeError('Could not inspect GitHub release state; publication stopped')
    return json.loads(response.stdout)



def generate_release_notes(tag, previous_tag, root=ROOT):
    """List every commit since the previous published release, including merged work."""
    version_tuple(tag)
    target = run('git', 'rev-parse', '--verify', f'refs/tags/{tag}^{{commit}}',
                 cwd=root, capture_output=True, text=True).stdout.strip()
    revision = target
    lines = [f'# Codex Resets {tag[1:]}', '',
             f'Release commit: [{target}](https://github.com/{REPO}/commit/{target})', '']
    if previous_tag:
        version_tuple(previous_tag)
        previous = run('git', 'rev-parse', '--verify', f'refs/tags/{previous_tag}^{{commit}}',
                       cwd=root, capture_output=True, text=True).stdout.strip()
        ancestor = subprocess.run(['git', 'merge-base', '--is-ancestor', previous, target], cwd=root)
        if ancestor.returncode != 0:
            raise ValueError('Previous published release is not an ancestor of this release')
        revision = f'{previous}..{target}'
        lines += [f'Changes since [{previous_tag}](https://github.com/{REPO}/releases/tag/{previous_tag}).',
                  f'[Full comparison](https://github.com/{REPO}/compare/{previous_tag}...{tag})', '']
    else:
        lines += ['First published release: includes all commits reachable from this tag.', '']
    output = run('git', 'log', '--reverse', '--topo-order', '-z', '--format=%H%x00%s', revision, '--',
                 cwd=root, capture_output=True, text=True).stdout
    fields = output.rstrip('\0').split('\0') if output else []
    commits = list(zip(fields[::2], fields[1::2]))
    lines += [f'## Commits ({len(commits)})', '']
    for commit, subject in commits:
        # Render commit subjects as plain text, not authored Markdown/HTML or mentions.
        subject = re.sub(r'([\\`*_{}\[\]()#+.!|~-])', r'\\\1', subject)
        subject = html.escape(subject, quote=False).replace('@', '&#64;')
        lines.append(f'- {subject} — [{commit}](https://github.com/{REPO}/commit/{commit})')
    if not commits:
        lines.append('No new commits since the previous release.')
    return '\n'.join(lines) + '\n'


def publish(tag):
    archive, feed = asset_paths(tag)
    if not archive.is_file() or not feed.is_file():
        raise ValueError('Both signed release assets must exist before publication')
    latest = github_release('latest')
    if latest and version_tuple(latest['tag_name']) >= version_tuple(tag):
        raise ValueError('Refusing to replace latest with the same or an older release')
    release = github_release('tags/' + tag)
    if release and not release['draft']:
        raise ValueError('Refusing to modify an already published release')
    notes = ROOT / 'dist/release-notes.md'
    notes.write_text(generate_release_notes(tag, latest['tag_name'] if latest else None, ROOT), encoding='utf-8')
    if not release:
        run('gh', 'release', 'create', tag, '--repo', REPO, '--verify-tag', '--draft',
            '--title', f'Codex Resets {tag[1:]}', '--notes-file', notes)
    else:
        run('gh', 'release', 'edit', tag, '--repo', REPO, '--notes-file', notes)
    # Draft remains invisible to /releases/latest until BOTH uploads succeed.
    run('gh', 'release', 'upload', tag, archive, '--repo', REPO, '--clobber')
    run('gh', 'release', 'upload', tag, feed, '--repo', REPO, '--clobber')
    run('gh', 'release', 'edit', tag, '--repo', REPO, '--draft=false', '--latest')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--sdk', type=Path, default=ROOT / 'build/sparkle/sdk')
    parser.add_argument('--publish', action='store_true')
    args = parser.parse_args()
    version_tuple(args.tag)
    metadata = json.loads(run('cargo', 'metadata', '--no-deps', '--format-version=1', cwd=ROOT,
                              capture_output=True, text=True).stdout)
    version = next(package['version'] for package in metadata['packages'] if package['name'] == 'codex-reset')
    if args.tag != 'v' + version:
        raise ValueError('Tag must match the checked-out Cargo package version')
    if args.publish:
        validate_tag(ROOT, args.tag)
        publish(args.tag)
    else:
        prepare(args.tag, args.sdk.resolve())


if __name__ == '__main__':
    try:
        main()
    except (ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        # Do not expose captured signing-tool stderr or any secret input.
        print(f'Release failed: {error}', file=sys.stderr)
        sys.exit(1)
