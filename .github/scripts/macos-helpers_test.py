import importlib.util
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('macos_helpers', Path(__file__).with_name('macos-helpers.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

OTHER = 'A' * 40
OWN = 'B' * 40


def keychain(subjects):
    def check_output(command, input=None, text=None):
        if command[:2] == ['security', 'find-identity']:
            return (f'  1) {OTHER} "Apple Development: Other (AAAAAAAAAA)"\n'
                    f'  2) {OWN} "Apple Development: Own (BBBBBBBBBB)"\n'
                    f'  3) {"C" * 40} "Developer ID Application: Own (T6QN7PY57N)"\n')
        if command[:2] == ['security', 'find-certificate']:
            digest = OTHER if 'Other' in command[-1] else OWN
            return f'SHA-1 hash: {digest}\n-----BEGIN CERTIFICATE-----\n{digest}\n-----END CERTIFICATE-----\n'
        return subjects[OTHER if OTHER in input else OWN]
    return check_output


class IdentityTests(unittest.TestCase):
    def test_開発用署名_projectのTeamのApple_Development証明書を選ぶ(self):
        # Given
        subjects = {OTHER: 'subject=CN=Other, OU=ZZZZZZZZZZ\n', OWN: 'subject=CN=Own, OU=T6QN7PY57N, O=Own\n'}
        # When
        with patch.object(module.subprocess, 'check_output', side_effect=keychain(subjects)):
            identity = module.development_identity()
        # Then
        self.assertEqual(identity, OWN)

    def test_開発用署名_Teamの証明書が無ければad_hocにする(self):
        # Given
        subjects = {OTHER: 'subject= /CN=Other/OU=ZZZZZZZZZZ\n', OWN: 'subject= /CN=Own/OU=YYYYYYYYYY\n'}
        # When
        with patch.object(module.subprocess, 'check_output', side_effect=keychain(subjects)):
            identity = module.development_identity()
        # Then
        self.assertEqual(identity, '-')


if __name__ == '__main__':
    unittest.main()
