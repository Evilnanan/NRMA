"""Register Battle Mage continuation aliases in each target's behavior graph."""
import argparse
import struct
from pathlib import Path

from build_onyx_behavior import Graph


def patch(raw, families):
    graph = Graph(raw)
    original = bytes(graph.data)
    allowed = set()
    _, strings = graph.objects('hkbBehaviorGraphStringData')[0]
    names_array = graph.q(strings + 88)
    names = graph.animation_names(names_array)
    clips = graph.objects('hkbClipGenerator')
    logical = {graph.text(graph.q(at + 152)): index for index, at in clips}
    selectors = graph.objects('CustomManualSelectorGenerator')
    aliases = {45020: [], 45030: [], 45120: [], 45130: []}
    # Families with two swing directions in battle_mage_poses.rs.
    directional = (401, 402, 409, 427, 437, 438, 479, 480, 494)
    if 'a576_045020' in logical:
        directional += (576,)
    for family, release in families:
        for action, nodes in aliases.items():
            source = f'a{family}_{release + (100 if action >= 45100 else 0):06d}'
            target = f'a{family}_{action:06d}'
            assert not any(graph.text(graph.q(at + 152)) == target for _, at in clips)
            (model, old), = (clip for clip in clips if graph.text(graph.q(clip[1] + 152)) == source)
            index = struct.unpack_from('<h', graph.data, old + 198)[0]
            path = graph.text(names[index])
            assert path.endswith(f'\\{source}.hkx')
            # Use a new logical binding. TAE ImportsHKX resolves the motion
            # to the original payload, as it does for native spell a513.
            target_path = path.replace(source, target)
            assert all(graph.text(name) != target_path for name in names)
            names.append(graph.string(names[index], target_path))
            assert len(names) <= 32768
            node, at = graph.append(graph.items[model][0], graph.data[old:old + 352])
            for offset, kind in graph.patch_type.items():
                if old <= offset < old + 352:
                    graph.patches[kind].append(at + offset - old)
            struct.pack_into('<Q', graph.data, at + 72,
                             graph.string(graph.q(old + 72), target + '_hkx_BattleMage'))
            struct.pack_into('<Q', graph.data, at + 152,
                             graph.string(graph.q(old + 152), target))
            struct.pack_into('<h', graph.data, at + 198, len(names) - 1)
            nodes.append(node)

    for action, nodes in aliases.items():
        side = 'Right' if action < 45100 else 'Left'
        stage = action % 100 // 10
        expected = f'MagicFire{side}{stage}_CMSG'
        alternate = action + (10 if stage == 2 else -10)
        # An animation-key offset can select the opposite swing without
        # re-entering the active Havok state. Preserve the native clip objects
        # and their own event tracks by sharing the other stage's child.
        reverse = [logical[f'a{family}_{alternate:06d}'] for family in directional]
        matched = set()
        arrays = {}
        for _, at in selectors:
            if struct.unpack_from('<ii', graph.data, at + 168) != (17, action):
                continue
            label = graph.text(graph.q(at + 72))
            assert label in (expected, expected + '_Upper') and label not in matched
            matched.add(label)
            array = graph.q(at + 152)
            if array not in arrays:
                refs = graph.refs(array)
                missing = nodes + [node for node in reverse if node not in refs]
                arrays[array] = graph.array(array, refs + missing) if missing else array
            struct.pack_into('<Q', graph.data, at + 152, arrays[array])
            allowed.update(range(at + 152, at + 160))
        assert matched == {expected, expected + '_Upper'}

    if families:
        struct.pack_into('<Q', graph.data, strings + 88, graph.array(names_array, names))
        allowed.update(range(strings + 88, strings + 96))
    changed = {i for i, (a, b) in enumerate(zip(original, graph.data)) if a != b}
    assert changed <= allowed
    result = graph.write()
    check = Graph(result)
    assert check.data == graph.data and check.items == graph.items and check.patches == graph.patches
    assert result[slice(*check.chunks[b'TYPE'])] == raw[slice(*graph.chunks[b'TYPE'])]
    assert all(check.q(i) < len(check.items) for offsets in check.patches.values() for i in offsets)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('families', nargs='+', metavar='FAMILY:RELEASE')
    args = parser.parse_args()
    families = [tuple(map(int, pair.split(':'))) for pair in args.families]
    args.output.write_bytes(patch(args.source.read_bytes(), families))
    print(f'PASS: {len(args.families) * 4} spell bindings registered in full-body and upper-body selectors')
