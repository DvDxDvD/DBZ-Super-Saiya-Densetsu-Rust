// Memory layout for DBZ Super Saiya Densetsu (LoROM SlowROM): the code runs in banks $00-$1F,
// while the ghidra-snes loader creates the ROM only in $80-$9F (+ a byte-mapped mirror of $00).
// This script:
//   - creates $00-$1F:8000-FFFF as *initialized* ROM blocks from the same FileBytes
//     (the decompiler folds constants only from initialized read-only memory);
//   - $00-$1F:0000-1FFF -> byte-mapped mirror of $7E:0000-1FFF (low WRAM);
//   - $01-$1F:2000-5FFF -> byte-mapped mirror of $00:2000-5FFF (I/O, unique MMIO labels);
//   - makes every ROM block read-only.
// Run it on a fresh import (it removes the loader's snes_low_ram and bank_00_rom_mirror).
// @category DBZ
import ghidra.app.script.GhidraScript;
import ghidra.program.database.mem.FileBytes;
import ghidra.program.model.address.Address;
import ghidra.program.model.address.AddressSpace;
import ghidra.program.model.mem.Memory;
import ghidra.program.model.mem.MemoryBlock;

public class DbzMirrors extends GhidraScript {

    private Memory mem;
    private AddressSpace bus;

    private Address a(long v) {
        return bus.getAddress(v);
    }

    private boolean mirror(String name, long start, long target, long len, boolean vol) throws Exception {
        if (mem.getBlock(a(start)) != null) {
            return false;
        }
        MemoryBlock b = mem.createByteMappedBlock(name, a(start), a(target), len, false);
        b.setRead(true);
        b.setWrite(true);
        b.setExecute(false);
        b.setVolatile(vol);
        return true;
    }

    @Override
    public void run() throws Exception {
        mem = currentProgram.getMemory();
        bus = currentProgram.getAddressFactory().getDefaultAddressSpace();
        FileBytes fb = mem.getAllFileBytes().get(0);
        int rom = 0, mirrors = 0;

        for (String old : new String[] { "snes_low_ram", "bank_00_rom_mirror" }) {
            MemoryBlock b = mem.getBlock(old);
            if (b != null) {
                mem.removeBlock(b, monitor);
                println("removed loader block: " + old);
            }
        }

        long romSize = fb.getSize();
        for (int bank = 0x00; bank <= 0x1F; bank++) {
            long base = (long) bank << 16;
            long off = (long) bank * 0x8000;
            if (off + 0x8000 <= romSize && mem.getBlock(a(base | 0x8000)) == null) {
                MemoryBlock b = mem.createInitializedBlock(String.format("bank_%02x_rom", bank), a(base | 0x8000),
                    fb, off, 0x8000, false);
                b.setRead(true);
                b.setWrite(false);
                b.setExecute(true);
                rom++;
            }
            if (mirror(String.format("bank_%02x_lowram_mirror", bank), base, 0x7E0000, 0x2000, false)) {
                mirrors++;
            }
            if (bank > 0 && mirror(String.format("bank_%02x_io_mirror", bank), base | 0x2000, 0x002000, 0x4000, true)) {
                mirrors++;
            }
        }

        int ro = 0;
        for (MemoryBlock b : mem.getBlocks()) {
            if (b.isExecute() && b.isWrite()) {
                b.setWrite(false);
                ro++;
            }
        }
        println("ROM banks $00-$1F created: " + rom + "; RAM/IO mirrors: " + mirrors + "; blocks made read-only: " + ro);
    }
}
