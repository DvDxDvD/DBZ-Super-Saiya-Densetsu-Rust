#!/usr/bin/env python3
"""Lists the story scripts ("scenes") of the cached ROM, one command per line.

    python tools/re/scenes.py [--rom canonical.sfc] [scene ...]      (scene ids in hexadecimal)

The format of the scripts and what the commands do is in docs/re/data/scenes.md; this file holds only
the lengths and names of the 51 commands (our own description), the game's data is read from the
cached ROM at run time and printed to stdout, never written to a tracked file (CONTRIBUTING.md).
Scripts live in bank 6; the pointer of scene n is the word at 06:D109 + 2n; a jump, call or "won"
address is a bank 6 address. The output marks jump targets with labels and shows, for the commands that
have one, the decoded meaning (flag, group, map, character...).
"""
import os
import sys

SCENES = 0x52  # scripts in the pointer table at 06:D109
TABLE = 0xD109
END = 0xDFCB  # first byte after the last script

# opcode: (name, operand bytes)
OPS = {
    0x00: ("start", 0), 0x01: ("end", 0), 0x02: ("picture", 1), 0x03: ("object", 5),
    0x04: ("move", 2), 0x05: ("text", 2), 0x06: ("sound", 1), 0x07: ("music", 1),
    0x08: ("nop", 0), 0x09: ("nop", 0), 0x0A: ("nop", 0), 0x0B: ("ifflag", 3),
    0x0C: ("setflag", 1), 0x0D: ("clrflag", 1), 0x0E: ("sprite", 6), 0x0F: ("move2", 2),
    0x10: ("wait", 1), 0x11: ("battle", 4), 0x12: ("jump", 2), 0x13: ("poke", 3),
    0x14: ("screen14", 0), 0x15: ("copyobj", 2), 0x16: ("redraw", 0), 0x17: ("camera", 1),
    0x18: ("screen18", 0), 0x19: ("place", 4), 0x1A: ("zero", 2), 0x1B: ("effect", 1),
    0x1C: ("addmember", 2), 0x1D: ("portrait", 3), 0x1E: ("fxsetup", 3), 0x1F: ("call", 2),
    0x20: ("return", 0), 0x21: ("mode", 1), 0x22: ("savepos", 0), 0x23: ("restpos", 0),
    0x24: ("fx", 2), 0x25: ("call2", 2), 0x26: ("return2", 0), 0x27: ("camera2", 1),
    0x28: ("fade", 1), 0x29: ("tomode", 1), 0x2A: ("rmobj", 1), 0x2B: ("join", 1),
    0x2C: ("gomap", 6), 0x2D: ("screen2d", 0), 0x2E: ("scene", 1), 0x2F: ("inc", 2),
    0x30: ("battle2", 2), 0x31: ("party", 0), 0x32: ("start", 0),
}


def default_rom():
    base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~/.local/share")
    return os.path.join(base, "dbz-ssd-recompiled", "cache", "rom", "canonical.sfc")


def main(argv):
    rom_path = default_rom()
    ids = []
    it = iter(argv)
    for arg in it:
        if arg == "--rom":
            rom_path = next(it)
        else:
            ids.append(int(arg, 16))
    rom = open(rom_path, "rb").read()

    def at(addr):  # bank 6, LoROM
        return rom[6 * 0x8000 + addr - 0x8000]

    def word(addr):
        return at(addr) | at(addr + 1) << 8

    pointers = {}
    for s in range(SCENES):
        pointers.setdefault(word(TABLE + 2 * s), []).append(s)
    starts = sorted(pointers)

    def commands(lo, hi):
        a = lo
        while a < hi:
            op = at(a)
            name, n = OPS.get(op, ("?%02X" % op, 0))
            yield a, op, name, bytes(at(a + 1 + i) for i in range(n))
            a += 1 + n

    labels = set()
    for i, lo in enumerate(starts):
        hi = starts[i + 1] if i + 1 < len(starts) else END
        for _, op, _, args in commands(lo, hi):
            if op in (0x12, 0x1F, 0x25):
                labels.add(args[0] | args[1] << 8)
            elif op == 0x0B:
                labels.add(args[1] | args[2] << 8)
            elif op in (0x11, 0x30):
                labels.add(args[-2] | args[-1] << 8)

    for i, lo in enumerate(starts):
        if ids and not any(s in ids for s in pointers[lo]):
            continue
        hi = starts[i + 1] if i + 1 < len(starts) else END
        print("== scene %s  [%04X]" % (",".join("%02X" % s for s in pointers[lo]), lo))
        for a, op, name, args in commands(lo, hi):
            note = ""
            if op in (0x12, 0x1F, 0x25):
                note = " -> %04X" % (args[0] | args[1] << 8)
            elif op == 0x0B:
                note = " flag %02X -> %04X" % (args[0], args[1] | args[2] << 8)
            elif op in (0x11, 0x30):
                note = " group %02X won -> %04X" % (args[0] if op == 0x11 else 0, args[-2] | args[-1] << 8)
            elif op == 0x13:
                note = " [%04X] = %02X" % (args[0] | args[1] << 8, args[2])
            elif op == 0x2C:
                note = " map %02X at (%d, %d)" % (args[1], args[2] | args[3] << 8, args[4] | args[5] << 8)
            elif op in (0x1C, 0x2B):
                note = " character %d" % args[0] + (" level %d" % args[1] if op == 0x1C else "")
            label = "L%04X:" % a if a in labels else ""
            print("  %04X %-7s %-9s %s%s" % (a, label, name, args.hex(" "), note))


if __name__ == "__main__":
    main(sys.argv[1:])
