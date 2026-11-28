"""Select a native continuation for accepted Battle Mage follow-ups."""
import argparse
import hashlib
from pathlib import Path
import re
import struct

from hks_bytecode import Reader, decode

QUERY = 990065
ACCEPTED = 990066
FULL_BODY = 990067
FUNCTION = 'ExecComboMagic'
LEFT_THIRD_UPDATE = 'MagicFireLeft3_Upper_onUpdate'
LEFT_CAST = 'Event_MagicFireLeft3'
LEFT_CANCEL = 'Event_MagicFireLeftCancel3'
NATIVE_CODE = 'd8f364c25dae4eb50647f3671e613206e0e7306d6e8e9fd62e05f490a1386545'
PREVIOUS_CODES = {
    'a8ebf93433ef008a954b4f28eee5061ac8ffafa8f7ffa28aceb3f94893c750f4',
    'd62156bc11c135c932317349bb960746eb28af2ef29e69471a4eaecb6e16c5f3',
    'c22e53eca42d20b5a6ba3bbdd3eaf1197056acbbe304dbf035080079da2addb7',
    'eafec1c7766dbb3ce9de4e1c4a5bade81fe1973ba4033779027ab738107008b5',
}
ORIGINAL = Path(__file__).resolve().parents[1] / 'assets/original/action/script/c0000.hks'
NATIVE_EVENTS = (
    '"W_RideMagicFireCombo1"', '"W_RideMagicFireCombo2"',
    'Event_MagicFireRight2', 'Event_MagicFireRight3',
    'Event_MagicFireLeft2', 'Event_MagicFireLeft3',
)


def abx(op, a, bx):
    return op << 25 | bx << 8 | a


def abc(op, a, b, c):
    return op << 25 | b << 17 | c << 8 | a


def assemble(function):
    """Return exact prototype edits, preserving native jump targets."""
    assert function['slots'] == 7 and struct.unpack('>I', function['header'][4:8])[0] == 4
    assert hashlib.sha256(function['code']).hexdigest() == NATIVE_CODE
    constants = function['constants']
    env = constants.index('env')
    query, accepted, one, two, three = range(len(constants), len(constants) + 5)
    full_body, zero, set_variable, locomotion, allbody = (
        three + 1, three + 2, three + 3, three + 6, three + 7)
    values = [float(QUERY), float(ACCEPTED), 1.0, 2.0, 3.0, float(FULL_BODY), 0.0,
              'SetVariable', None, None, 'LocomotionState', 'ALLBODY', None, None]

    def constant(value):
        if value is None:
            return b'\x00'
        if isinstance(value, str):
            raw = value.encode('ascii') + b'\x00'
            return b'\x04' + struct.pack('>Q', len(raw)) + raw
        return b'\x03' + struct.pack('>f', value)

    encoded = b''.join(map(constant, values))
    padding = -len(encoded) % 4
    encoded += b'\x00' * padding
    values += [None] * padding

    # The query can authorize a different family or reject the new clips for
    # ordinary spells sharing their animation family. Then use native checks.
    reject = [abx(25, 5, two), abc(4, 0, 4, 5), abx(28, 0, 0x10001),
              abx(91, 4, constants.index('FALSE')), abc(9, 4, 2, 0)]
    is_true = [abx(25, 5, one), abc(4, 0, 4, 5)]
    prefix = [abx(91, 4, env), abx(76, 20, env + 1), abx(25, 5, query),
              abc(7, 6, 3, 0), abc(2, 4, 3, 2)] + reject + is_true + [
              abx(28, 0, 0x10000), abc(7, 2, 3, 0)]
    original = [w[0] for w in struct.iter_unpack('>I', function['code'])]
    transitions = [i for i, word in enumerate(original)
                   if decode(word)['op'] == 91 and constants[decode(word)['bx']]
                   in ('ExecEventHalfBlend', 'ExecEventAllBody')]
    assert transitions == [32, 49, 74, 93, 116, 135]
    inserts = {0: prefix}
    for second, third in zip(transitions[::2], transitions[1::2]):
        for pc, other, stage, alternative in ((second, third, two, three), (third, second, three, two)):
            mounted = pc < 60
            call = pc + (3 if mounted else 5)
            assert decode(original[call])['op'] == 2
            alt_argument = original[other + 2:other + (3 if mounted else 4)]
            assert decode(alt_argument[0])['a'] == 5
            inserts[call] = [
                abx(91, 7, env), abx(76, 20, env + 1),
                abx(25, 8, accepted), abx(25, 9, stage), abc(2, 7, 3, 2),
                abx(25, 8, alternative), abc(4, 0, 7, 8),
                abx(28, 0, 0x10000 + len(alt_argument) - 1)
            ] + alt_argument
            if not mounted:
                # The previous spell can be moving while the new one needs
                # its planted cast. Select both layers and clear locomotion.
                fixed = [abx(91, 7, set_variable), abx(76, 20, set_variable + 1),
                         abx(25, 8, locomotion), abx(25, 9, zero), abc(2, 7, 3, 1),
                         abx(91, 6, allbody), abx(76, 20, allbody + 1)]
                inserts[call] += [abx(91, 7, env), abx(76, 20, env + 1),
                                  abx(25, 8, full_body), abc(2, 7, 2, 2),
                                  abx(25, 8, one), abc(4, 0, 7, 8),
                                  abx(28, 0, 0x10000 + len(fixed) - 1)] + fixed

    def before(pc):
        return pc + sum(len(words) for pos, words in inserts.items() if pos < pc)

    def after(pc):
        return before(pc) + len(inserts.get(pc, ()))

    generated = []
    lines = []
    for pc, word in enumerate(original):
        line = function['lines'][pc * 4:pc * 4 + 4]
        additions = inserts.get(pc, ())
        generated.extend(additions)
        lines.extend([line] * (len(additions) + 1))
        d = decode(word)
        if d['op'] == 28:
            dest = pc + 2 + d['bx'] - 0x10000
            assert 0 <= dest <= len(original)
            word = abx(28, d['a'], 0x10000 + before(dest) - after(pc) - 2)
        generated.append(word)
    code = b''.join(struct.pack('>I', word) for word in generated)

    def count(key, value):
        return (function[key], 4, struct.pack('>I', value))

    edits = [
        count('slots_offset', 10),
        count('code_count_offset', len(generated)),
        (function['code_offset'], len(function['code']), code),
        count('constants_count_offset', len(constants) + len(values)),
        (function['constants_end'], 0, encoded),
        count('line_count_offset', len(generated)),
        (function['lines_offset'], len(function['lines']), b''.join(lines)),
    ]
    for _, a, start, b, end in function['locals']:
        edits.extend([(a, 4, struct.pack('>I', before(start))),
                      (b, 4, struct.pack('>I', before(end)))])
    return edits, code, constants + values


def mmv_edits(body):
    newline = '\r\n' if '\r\n' in body else '\n'
    gate = '    if lastUsedMagicAnim ~= magicAnim then' + newline
    at = body.index(gate)
    assert body.count(gate) == 1
    yield at, at + len(gate), newline.join((
        f'    local battleMage = env({QUERY}, magicAnim)',
        '    if battleMage == 2 then', '        return FALSE', '    end',
        '    if lastUsedMagicAnim ~= magicAnim and battleMage ~= 1 then', ''))
    found = list(re.finditer(r'(?m)^( +)ExecEvent(?:AllBody|HalfBlend)\(', body))
    assert len(found) == len(NATIVE_EVENTS)
    for index, match in enumerate(found):
        event, alternative = NATIVE_EVENTS[index], NATIVE_EVENTS[index ^ 1]
        assert body[match.end():].startswith(event)
        at = match.end()
        stage = 2 + index % 2
        decision = f'env({ACCEPTED}, {stage}) == {5 - stage} and {alternative} or {event}'
        if index < 2:
            yield at, at + len(event), decision
            continue
        # Evaluate the accepted query before inspecting the selected cast.
        end = body.index(')', at) + 1
        assert body[at:end] == event + ', blend_type)'
        start = match.start()
        indent = match.group(1)
        yield start, end, newline.join((
            f'{indent}local event = {decision}',
            f'{indent}if env({FULL_BODY}) == 1 then',
            f'{indent}    SetVariable("LocomotionState", 0)',
            f'{indent}    blend_type = ALLBODY',
            f'{indent}end',
            f'{indent}ExecEventHalfBlend(event, blend_type)'))


def patch_combo(source):
    reader = Reader(source)
    reader.function()
    match, = [f for f in reader.functions if f['name'] == FUNCTION]
    if QUERY in match['constants']:
        template = Reader(ORIGINAL.read_bytes())
        template.function()
        original, = [f for f in template.functions if f['name'] == FUNCTION]
        _, code, constants = assemble(original)
        if (match['code'], match['constants']) == (code, constants):
            return source
        if hashlib.sha256(match['code']).hexdigest() not in PREVIOUS_CODES:
            raise ValueError('Battle Mage combo script has an unknown revision')
        native = patch_binary(template.data)
        clean = Reader(native)
        clean.function()
        patched, = [f for f in clean.functions if f['name'] == FUNCTION]
        assert not match['children'] and not patched['children']
        assert match['start'] % 4 == patched['start'] % 4
        output = source[:match['start']] + native[patched['start']:patched['end']] + source[match['end']:]
    else:
        edits, code, constants = assemble(match)
        output = source
        for offset, size, value in sorted(edits, reverse=True):
            output = output[:offset] + value + output[offset + size:]
    rebuilt = Reader(output)
    rebuilt.function()
    assert len(reader.functions) == len(rebuilt.functions)
    assert source[reader.p:] == output[rebuilt.p:]
    for old, new in zip(reader.functions, rebuilt.functions):
        assert old['name'] == new['name']
        if old['name'] == FUNCTION:
            assert (new['code'], new['constants']) == (code, constants)
        elif old['id']:
            assert source[old['start']:old['end']] == output[new['start']:new['end']]
        else:
            assert (old['code'], old['constants']) == (new['code'], new['constants'])
    return output


def sync_left_third(source):
    reader = Reader(source)
    reader.function()
    current, = (f for f in reader.functions if f['name'] == LEFT_THIRD_UPDATE)
    constants = current['constants']
    words = [item[0] for item in struct.iter_unpack('>I', current['code'])]
    call, = (i for i, word in enumerate(words)
             if word == abx(91, 3, constants.index('HalfBlendLowerCommonFunction')))
    if LEFT_CAST in constants:
        assert words[call + 2] == abx(91, 4, constants.index(LEFT_CAST))
        return source
    old = constants.index(LEFT_CANCEL)
    assert words[call + 2:call + 4] == [abx(91, 4, old), abx(76, 20, old + 1)]
    assert words[call + 7] == abc(2, 3, 4, 2)
    # The actual cancel branches keep their event. On a locomotion stop the
    # lower layer rejoins the cast already running on the upper layer.
    name = LEFT_CAST.encode('ascii') + b'\x00'
    extra = b'\x04' + struct.pack('>Q', len(name)) + name + b'\x00\x00'
    padding = -len(extra) % 4
    slot = len(constants)
    code_at = current['code_offset'] + (call + 2) * 4
    count_at = current['constants_count_offset']
    end = current['constants_end']
    output = (source[:code_at] + struct.pack('>II', abx(91, 4, slot), abx(76, 20, slot + 1))
              + source[code_at + 8:count_at] + struct.pack('>I', slot + 3 + padding)
              + source[count_at + 4:end] + extra + b'\x00' * padding + source[end:])
    check = Reader(output)
    check.function()
    assert source[reader.p:] == output[check.p:]
    for old_func, new_func in zip(reader.functions[1:], check.functions[1:], strict=True):
        assert old_func['name'] == new_func['name']
        if old_func is not current:
            assert source[old_func['start']:old_func['end']] == output[new_func['start']:new_func['end']]
    return output


def patch_binary(source):
    return sync_left_third(patch_combo(source))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('script', type=Path)
    args = parser.parse_args()
    output = patch_binary(args.script.read_bytes())
    args.script.write_bytes(output)
    print('PASS: native combo windows and direct continuation events verified')
