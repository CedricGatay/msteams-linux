"""Offline asset checks for the original generator icon (no image dependencies)."""
import pathlib
import struct
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


class GeneratorArtworkTests(unittest.TestCase):
    def test_png_assets_have_expected_sizes_and_alpha(self):
        for path, size in [('dist/generator.png', 128),
                           ('src-tauri/icons/generator/icon.png', 1024)]:
            data = (ROOT / path).read_bytes()
            self.assertEqual(data[:8], b'\x89PNG\r\n\x1a\n')
            self.assertEqual(data[12:16], b'IHDR')
            self.assertEqual(struct.unpack('>II', data[16:24]), (size, size))
            self.assertEqual(data[25], 6)  # RGBA, including transparent macOS margins.

    def test_icns_is_complete_and_includes_retina_representations(self):
        data = (ROOT / 'src-tauri/icons/generator/icon.icns').read_bytes()
        self.assertEqual(data[:4], b'icns')
        self.assertEqual(struct.unpack('>I', data[4:8])[0], len(data))
        position, representations = 8, set()
        while position < len(data):
            kind, length = struct.unpack('>4sI', data[position:position + 8])
            self.assertGreaterEqual(length, 8)
            representations.add(kind)
            position += length
        self.assertEqual(position, len(data))
        self.assertTrue({b'ic09', b'ic10'}.issubset(representations))  # 512 and 1024 pixels.

    def test_generator_artwork_is_distinct_from_each_service(self):
        png = (ROOT / 'dist/generator.png').read_bytes()
        icns = (ROOT / 'src-tauri/icons/generator/icon.icns').read_bytes()
        for service in ['teams', 'outlook', 'word', 'excel', 'powerpoint']:
            self.assertNotEqual(png, (ROOT / f'dist/services/{service}.png').read_bytes())
            icon = 'icon.icns' if service == 'teams' else f'{service}/icon.icns'
            self.assertNotEqual(icns, (ROOT / 'src-tauri/icons' / icon).read_bytes())


if __name__ == '__main__':
    unittest.main()
