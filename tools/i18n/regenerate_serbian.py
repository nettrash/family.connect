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

from serbian import SR, SR_COUNTS, SR_PLURAL, to_latin  # noqa: E402
from serbian_android import ANDROID_ONLY  # noqa: E402

XCSTRINGS = os.path.join(ROOT, 'ios/FamilyConnect/Localizable.xcstrings')
ANDROID_RES = os.path.join(ROOT, 'android/app/src/main/res')

# Serbian's CLDR plural categories, in the order this file has always written
# them (Xcode's own are alphabetical; both survive the round-trip).
ORDER = ['one', 'few', 'other']

# The poll counts, whose Serbian lives in SR_COUNTS as one template per
# category. Each entry says how to render that template for one platform.
#
# Apple: a count that is the ONLY argument is a plural variation; a count that
# shares the string with another argument has to be an explicit substitution,
# or xcstringstool cannot tell which argument the rule reads and says so.
APPLE_COUNTS = [
    # catalogue key, SR_COUNTS entry, the format the substitution sits in
    # (None for a plain plural variation), argNum, placeholders
    ('%lld voted', 'votes', None, None, {'n': '%lld'}),
    ('%lld of %lld voted', 'votes_of', '%#@arg1@', 1,
     {'n': '%arg', 'total': '%2$lld'}),
    ('%@. %lld votes', 'votes', '%1$@. %#@arg2@', 2, {'n': '%arg'}),
    ('%lld new messages', 'new_messages', None, None, {'n': '%lld'}),
    ('%lld unread', 'unread', None, None, {'n': '%lld'}),
]

# Android: <plurals>, and the quantity is always the first argument.
ANDROID_COUNTS = {
    's_voted_total': ('votes', {'n': '%1$d'}),
    's_voted_of_family': ('votes_of', {'n': '%1$d', 'total': '%2$d'}),
    's_n_new_messages': ('new_messages', {'n': '%1$d'}),
    's_use_at_least_characters': ('characters', {'n': '%1$d'}),
}


def rendered(entry, placeholders):
    """SR_COUNTS[entry] with the placeholders filled in, category by category."""
    return [(c, SR_COUNTS[entry][c].format(**placeholders)) for c in ORDER]


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


def substitution(fmt, arg_num, pairs):
    """A plural on an argument that is not the string's only one.

    Xcode's shape: the unit keeps a `%#@argN@` token and the variations live
    under a substitution that names the argument explicitly.
    """
    return collections.OrderedDict([
        ('stringUnit', collections.OrderedDict(
            [('state', 'translated'), ('value', fmt)])),
        ('substitutions', collections.OrderedDict([
            ('arg%d' % arg_num, collections.OrderedDict([
                ('argNum', arg_num),
                ('formatSpecifier', 'lld'),
                ('variations', plural(pairs)['variations']),
            ]))])),
    ])


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
    members = d['strings']['%lld members']
    put(members, 'sr', plural([(c, SR_PLURAL[c]) for c in ORDER]))
    put(members, 'sr-Latn', plural([(c, to_latin(SR_PLURAL[c])) for c in ORDER]))

    for key, name, fmt, arg_num, holes in APPLE_COUNTS:
        entry = d['strings'][key]
        pairs = rendered(name, holes)
        latin = [(c, to_latin(v)) for c, v in pairs]
        if fmt is None:
            put(entry, 'sr', plural(pairs))
            put(entry, 'sr-Latn', plural(latin))
        else:
            put(entry, 'sr', substitution(fmt, arg_num, pairs))
            put(entry, 'sr-Latn', substitution(fmt, arg_num, latin))

    io.open(XCSTRINGS, 'w', encoding='utf-8').write(dump(d))
    json.loads(io.open(XCSTRINGS, encoding='utf-8').read())
    print('ios: %d strings + %d plurals in sr and sr-Latn'
          % (len(SR), 1 + len(APPLE_COUNTS)))


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

    # A <plurals> is a different resource type, so the scan above cannot see
    # one — and a plural nobody rendered would be a missing translation, which
    # fails the Android build. Fail here instead, where the fix is obvious.
    quantities = re.findall(r'<plurals name="([^"]+)"', src)
    unknown = [n for n in quantities if n not in ANDROID_COUNTS]
    if unknown:
        raise SystemExit('no Serbian plural for %s — add it to ANDROID_COUNTS '
                         'and SR_COUNTS' % ', '.join(unknown))

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
        for name in quantities:
            entry, holes = ANDROID_COUNTS[name]
            body += '    <plurals name="%s">\n' % name
            for category, value in rendered(entry, holes):
                body += '        <item quantity="%s">%s</item>\n' % (
                    category, escape(convert(value)))
            body += '    </plurals>\n'
        body += '</resources>\n'
        io.open(os.path.join(path, 'strings.xml'), 'w', encoding='utf-8').write(body)
        print('android: %d strings + %d plurals in %s'
              % (len(resolved), len(quantities), folder))


if __name__ == '__main__':
    write_apple()
    write_android()
