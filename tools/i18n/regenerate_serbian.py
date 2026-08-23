#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Regenerate the Serbian resources for both platforms from serbian.py.

Serbian Cyrillic is the SOURCE OF TRUTH; Serbian Latin is transliterated from
it, never translated separately — the mapping is 1:1, so the two scripts
cannot drift apart. Edit the Cyrillic in serbian.py (and serbian_android.py
for the strings Android does not share with iOS), then run this.

    python3 tools/i18n/regenerate_serbian.py

Writes:
    ios/FamilyConnect/Localizable.xcstrings      sr + sr-Latn localizations
    android/app/src/main/res/values-sr/          Cyrillic
    android/app/src/main/res/values-b+sr+Latn/   Latin

The .xcstrings writer is byte-lossless: it asserts that re-serialising the
untouched file reproduces it exactly before changing anything, so a run that
adds one string does not reformat five thousand lines. See the xcstrings
notes in the repo history for the four rules that make that hold.
"""

import collections
import io
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)

from serbian import SR, SR_PLURAL, to_latin  # noqa: E402
from serbian_android import ANDROID_ONLY  # noqa: E402

XCSTRINGS = os.path.join(ROOT, 'ios/FamilyConnect/Localizable.xcstrings')
ANDROID_RES = os.path.join(ROOT, 'android/app/src/main/res')


# ---------------------------------------------------------------- xcstrings

def dump(obj, indent=0):
    """Xcode's String Catalog style, verified byte-identical on a round-trip."""
    pad = '  ' * indent
    inner = '  ' * (indent + 1)
    if isinstance(obj, dict):
        if not obj:
            # Xcode writes an empty object with a blank line inside it.
            return '{\n\n' + pad + '}'
        return ('{\n' + ',\n'.join(
            inner + json.dumps(k, ensure_ascii=False) + ' : ' + dump(v, indent + 1)
            for k, v in obj.items()) + '\n' + pad + '}')
    return json.dumps(obj, ensure_ascii=False)


def unit(value):
    return collections.OrderedDict([
        ('stringUnit', collections.OrderedDict(
            [('state', 'translated'), ('value', value)]))])


def plural(pairs):
    return collections.OrderedDict([('variations', collections.OrderedDict([
        ('plural', collections.OrderedDict(
            (c, collections.OrderedDict([('stringUnit', collections.OrderedDict(
                [('state', 'translated'), ('value', v)]))]))
            for c, v in pairs))]))])


def put(entry, lang, value):
    locs = entry.setdefault('localizations', collections.OrderedDict())
    locs[lang] = value
    entry['localizations'] = collections.OrderedDict(sorted(locs.items()))


def write_apple():
    original = io.open(XCSTRINGS, encoding='utf-8').read()
    d = json.loads(original, object_pairs_hook=collections.OrderedDict)
    assert dump(d) == original, 'xcstrings serializer is not lossless — fix it before writing'

    for english, cyrillic in SR.items():
        entry = d['strings'][english]
        put(entry, 'sr', unit(cyrillic))
        put(entry, 'sr-Latn', unit(to_latin(cyrillic)))

    # Serbian's CLDR plural categories are one/few/other.
    order = ['one', 'few', 'other']
    members = d['strings']['%lld members']
    put(members, 'sr', plural([(c, SR_PLURAL[c]) for c in order]))
    put(members, 'sr-Latn', plural([(c, to_latin(SR_PLURAL[c])) for c in order]))

    io.open(XCSTRINGS, 'w', encoding='utf-8').write(dump(d))
    json.loads(io.open(XCSTRINGS, encoding='utf-8').read())
    print('ios: %d strings + 1 plural in sr and sr-Latn' % len(SR))


# ------------------------------------------------------------------ android

def unescape(value):
    return (value.replace("\\'", "'").replace('&amp;', '&')
                 .replace('&lt;', '<').replace('&gt;', '>'))


def escape(value):
    """Android eats a bare apostrophe and reads & < > as markup."""
    return (value.replace('&', '&amp;').replace('<', '&lt;')
                 .replace('>', '&gt;').replace("'", "\\'"))


def write_android():
    src = io.open(os.path.join(ANDROID_RES, 'values/strings.xml'), encoding='utf-8').read()
    rows = re.findall(r'<string name="([^"]+)"([^>]*)>(.*?)</string>', src, re.S)
    todo = [(n, v) for n, a, v in rows if 'translatable="false"' not in a]

    # Anything Android words the same as iOS reuses the iOS translation —
    # the two platforms must not say the same sentence differently.
    by_english = {k.lower(): v for k, v in SR.items()}
    resolved = []
    for name, raw in todo:
        english = unescape(raw)
        if name in ANDROID_ONLY:
            resolved.append((name, ANDROID_ONLY[name]))
        elif english in SR:
            resolved.append((name, SR[english]))
        elif english.lower() in by_english:
            resolved.append((name, by_english[english.lower()]))
        else:
            raise SystemExit('no Serbian for %s = %r — add it to serbian_android.py'
                             % (name, english))

    header = ('<?xml version="1.0" encoding="utf-8"?>\n'
              '<!-- %s. Informal register — see values/strings.xml.%s -->\n'
              '<resources>\n')
    for folder, label, note, convert in (
        ('values-sr', 'sr', '', lambda s: s),
        ('values-b+sr+Latn', 'sr-Latn',
         ' Transliterated from values-sr, never translated separately.', to_latin),
    ):
        path = os.path.join(ANDROID_RES, folder)
        os.makedirs(path, exist_ok=True)
        body = header % (label, note)
        for name, value in resolved:
            body += '    <string name="%s">%s</string>\n' % (name, escape(convert(value)))
        body += '</resources>\n'
        io.open(os.path.join(path, 'strings.xml'), 'w', encoding='utf-8').write(body)
        print('android: %d strings in %s' % (len(resolved), folder))


if __name__ == '__main__':
    write_apple()
    write_android()
