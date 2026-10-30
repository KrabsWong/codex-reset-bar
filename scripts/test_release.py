import base64
import unittest
from release import FEED_URL, validate_bundle, version_tuple


class ReleaseValidationTests(unittest.TestCase):
    def setUp(self):
        self.key = base64.b64encode(bytes(range(32))).decode()
        self.info = dict(CFBundleIdentifier='com.krabs.codex-reset', CFBundleVersion='0.2.0',
                         CFBundleShortVersionString='0.2.0', SUFeedURL=FEED_URL,
                         SUPublicEDKey=self.key, SUVerifyUpdateBeforeExtraction=True,
                         SURequireSignedFeed=True)

    def test_accepts_matching_release(self):
        validate_bundle(self.info, 'v0.2.0', self.key)
        self.assertGreater(version_tuple('v0.10.0'), version_tuple('v0.9.0'))

    def test_rejects_wrong_bundle_version(self):
        self.info['CFBundleVersion'] = '5'
        with self.assertRaises(ValueError):
            validate_bundle(self.info, 'v0.2.0', self.key)

    def test_rejects_missing_or_changed_public_key(self):
        for key in ['', base64.b64encode(bytes(32)).decode()]:
            with self.assertRaises(ValueError):
                validate_bundle(self.info, 'v0.2.0', key)

    def test_rejects_unsigned_feed_and_test_feed(self):
        for name, value in [('SURequireSignedFeed', False), ('SUFeedURL', 'http://127.0.0.1/appcast.xml')]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                validate_bundle(dict(self.info, **{name: value}), 'v0.2.0', self.key)

    def test_rejects_prerelease_or_non_version_tags(self):
        for tag in ['v0.2.0-beta', 'v01.2.0', '0.2.0', 'latest', 'v0.2.0; echo unsafe']:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                version_tuple(tag)


if __name__ == '__main__':
    unittest.main()
