// Imports the Mesen2 traces (trace.lua) into the DBZ program:
//  1. DP = 0 on all ROM code (checked at runtime: D is always 0 at every sampled PC);
//     PCs with D other than 0 are excluded and reported.
//  2. DBR for each executed PC where a single value was observed.
//  3. ctx_MF / ctx_XF / ctx_EF for each executed PC (first mode observed; multi-mode PCs get a
//     bookmark).
//  4. Disassembles from every executed PC and creates functions at the vectors and at the
//     observed JSR/JSL targets.
// Arguments: one or more trace.lua output folders separated by + (cmd.exe splits arguments on ;).
// Required: there is no default path. Run DbzMirrors.java before this script, then auto-analysis.
// Incremental: it can be re-run on an already analysed program (also from the GUI via MCP). If an
// executed PC falls inside an instruction disassembled with a different M/X/E mode, the
// instruction is cleared and disassembled again with the observed mode.
// @category DBZ
import ghidra.app.cmd.disassemble.DisassembleCommand;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.address.AddressSet;
import ghidra.program.model.address.AddressSpace;
import ghidra.program.model.lang.Register;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.ContextChangeException;
import ghidra.program.model.listing.ProgramContext;
import ghidra.program.model.mem.Memory;
import ghidra.program.model.mem.MemoryBlock;
import ghidra.program.model.symbol.SourceType;

import java.io.BufferedReader;
import java.io.File;
import java.io.FileReader;
import java.math.BigInteger;
import java.util.*;

public class DbzApplyTrace extends GhidraScript {

    private static final String BM = "DBZ trace";

    private static final class PcInfo {
        final Set<String> modes = new TreeSet<>();   // "MXE/D/DBR"
        String first;
        long hits;
    }

    private AddressSpace bus;
    private Memory mem;
    private int redone;

        private void setCtx(ProgramContext ctx, Register r, Address addr, BigInteger v) throws Exception {
        if (getInstructionAt(addr) != null && v.equals(ctx.getValue(r, addr, false))) {
            return;   // instruction already disassembled with the same mode
        }
        try {
            ctx.setValue(r, addr, addr, v);
        } catch (ContextChangeException e) {
            Instruction ins = getInstructionContaining(addr);
            if (ins != null) {
                clearListing(ins.getMinAddress(), ins.getMaxAddress());
                redone++;
            }
            ctx.setValue(r, addr, addr, v);
        }
    }

    private Address a(long v) {
        return bus.getAddress(v);
    }

    private boolean isRomCode(Address addr) {
        MemoryBlock b = mem.getBlock(addr);
        return b != null && b.isExecute() && b.getName().contains("rom");
    }

    private List<String[]> readCsv(File f) throws Exception {
        List<String[]> rows = new ArrayList<>();
        if (!f.exists()) {
            return rows;
        }
        try (BufferedReader r = new BufferedReader(new FileReader(f))) {
            String line = r.readLine();   // header
            while ((line = r.readLine()) != null) {
                if (!line.isBlank()) {
                    rows.add(line.split(","));
                }
            }
        }
        return rows;
    }

    @Override
    public void run() throws Exception {
        bus = currentProgram.getAddressFactory().getDefaultAddressSpace();
        mem = currentProgram.getMemory();
        if (getScriptArgs().length == 0) {
            throw new IllegalArgumentException("usage: DbzApplyTrace.java <trace_dir>[+<trace_dir>...]");
        }
        String arg = getScriptArgs()[0];

        Map<Long, PcInfo> pcs = new TreeMap<>();
        Map<Long, String> callTargets = new TreeMap<>();
        Map<Long, String> jumpTargets = new TreeMap<>();
        for (String dir : arg.split("[;+]")) {
            for (String[] c : readCsv(new File(dir, "exec.csv"))) {
                long pc = Long.parseLong(c[0], 16);
                PcInfo p = pcs.computeIfAbsent(pc, k -> new PcInfo());
                String first = c[1] + c[2] + c[3] + "/" + Integer.parseInt(c[4], 16) + "/" + Integer.parseInt(c[5], 16);
                if (p.first == null) {
                    p.first = first;
                }
                p.hits += Long.parseLong(c[6]);
                p.modes.addAll(Arrays.asList(c[7].split(";")));
            }
            for (String[] c : readCsv(new File(dir, "calls.csv"))) {
                callTargets.put(Long.parseLong(c[0], 16), c[2]);
            }
            for (String[] c : readCsv(new File(dir, "jumps.csv"))) {
                jumpTargets.put(Long.parseLong(c[0], 16), c[2]);
            }
        }
        println("PCs executed: " + pcs.size() + ", call targets: " + callTargets.size() + ", jump targets: " + jumpTargets.size());

        ProgramContext ctx = currentProgram.getProgramContext();
        Register dp = currentProgram.getRegister("DP");
        Register dbr = currentProgram.getRegister("DBR");
        Register mf = currentProgram.getRegister("ctx_MF");
        Register xf = currentProgram.getRegister("ctx_XF");
        Register ef = currentProgram.getRegister("ctx_EF");

        // 1. DP = 0 on all executable ROM blocks
        Set<Long> dpExceptions = new TreeSet<>();
        for (Map.Entry<Long, PcInfo> e : pcs.entrySet()) {
            for (String m : e.getValue().modes) {
                if (!m.split("/")[1].equals("0")) {
                    dpExceptions.add(e.getKey());
                }
            }
        }
        for (MemoryBlock b : mem.getBlocks()) {
            if (b.isExecute() && b.getName().contains("rom")) {
                ctx.setValue(dp, b.getStart(), b.getEnd(), BigInteger.ZERO);
            }
        }
        for (long pc : dpExceptions) {
            ctx.remove(a(pc), a(pc), dp);
            createBookmark(a(pc), BM, "D other than 0 observed at runtime");
        }
        println("DP=0 applied; exceptions: " + dpExceptions.size());

        // 2-3. DBR and M/X/E flags for each executed PC
        int dbrSet = 0, multiMode = 0, skipped = 0;
        AddressSet toDisasm = new AddressSet();
        for (Map.Entry<Long, PcInfo> e : pcs.entrySet()) {
            Address addr = a(e.getKey());
            if (!isRomCode(addr)) {
                skipped++;   // code in RAM or outside the ROM blocks
                continue;
            }
            PcInfo p = e.getValue();
            Set<String> dbrs = new TreeSet<>();
            Set<String> flags = new TreeSet<>();
            for (String m : p.modes) {
                String[] parts = m.split("/");
                flags.add(parts[0]);
                dbrs.add(parts[2]);
            }
            if (dbrs.size() == 1) {
                ctx.setValue(dbr, addr, addr, new BigInteger(dbrs.iterator().next()));
                dbrSet++;
            }
            String f = p.first.split("/")[0];
            setCtx(ctx, mf, addr, BigInteger.valueOf(f.charAt(0) - '0'));
            setCtx(ctx, xf, addr, BigInteger.valueOf(f.charAt(1) - '0'));
            setCtx(ctx, ef, addr, BigInteger.valueOf(f.charAt(2) - '0'));
            if (flags.size() > 1) {
                multiMode++;
                createBookmark(addr, BM, "PC executed with several M/X/E modes: " + flags);
            }
            toDisasm.add(addr);
        }
        println("DBR set on " + dbrSet + " PCs; multi-mode PCs: " + multiMode + "; PCs outside ROM skipped: " + skipped
            + "; instructions redone for a different mode: " + redone);

        // 4. Disassemble and create functions
        DisassembleCommand dis = new DisassembleCommand(toDisasm, null, true);
        dis.applyTo(currentProgram, monitor);

        int created = 0;
        Map<Long, String> entries = new TreeMap<>();
        entries.put(readWord(0x00FFFC), "RESET");
        entries.put(readWord(0x00FFEA), "NMI");
        entries.put(readWord(0x00FFEE), "IRQ");
        for (long t : callTargets.keySet()) {
            entries.putIfAbsent(t, null);
        }
        for (Map.Entry<Long, String> e : entries.entrySet()) {
            Address addr = a(e.getKey());
            if (!isRomCode(addr)) {
                continue;
            }
            if (getInstructionAt(addr) == null) {
                disassemble(addr);
            }
            Function fn = getFunctionAt(addr);
            if (fn == null) {
                fn = createFunction(addr, e.getValue());
                if (fn != null) {
                    created++;
                }
            } else if (e.getValue() != null) {
                fn.setName(e.getValue(), SourceType.USER_DEFINED);
            }
        }
        int jt = 0;
        for (long t : jumpTargets.keySet()) {
            Address addr = a(t);
            if (isRomCode(addr) && getInstructionAt(addr) != null) {
                createBookmark(addr, BM, "indirect jump target (" + jumpTargets.get(t) + ")");
                jt++;
            }
        }

        // Semantic check of ctx_MF: LDA # with 8-bit M must be 2 bytes long.
        int checked = 0, bad = 0;
        for (Map.Entry<Long, PcInfo> e : pcs.entrySet()) {
            Instruction ins = getInstructionAt(a(e.getKey()));
            if (ins != null && (ins.getByte(0) & 0xFF) == 0xA9 && e.getValue().modes.size() == 1) {
                checked++;
                int expected = e.getValue().first.charAt(0) == '1' ? 2 : 3;
                if (ins.getLength() != expected) {
                    bad++;
                }
            }
        }
        println("functions created: " + created + "; jump targets marked: " + jt
            + "; LDA# check: " + checked + " verified, " + bad + " wrong");
    }

    private long readWord(long addr) throws Exception {
        return (mem.getByte(a(addr)) & 0xFF) | ((mem.getByte(a(addr + 1)) & 0xFF) << 8);
    }
}
