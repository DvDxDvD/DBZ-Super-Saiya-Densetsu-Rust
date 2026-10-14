// Prints the memory layout, the function count and the context registers (diagnostic).
// @category DBZ
import ghidra.app.script.GhidraScript;
import ghidra.program.model.mem.*;

public class DbzInfo extends GhidraScript {
    @Override
    public void run() throws Exception {
        for (MemoryBlock b : currentProgram.getMemory().getBlocks()) {
            String src = "";
            for (MemoryBlockSourceInfo si : b.getSourceInfos()) {
                src += si.getMappedRange().map(r -> " -> " + r).orElse("")
                    + si.getFileBytes().map(fb -> " file@" + Long.toHexString(si.getFileBytesOffset())).orElse("");
            }
            println(String.format("%-24s %s-%s %s init=%s x=%s%s", b.getName(), b.getStart(), b.getEnd(),
                b.getType(), b.isInitialized(), b.isExecute(), src));
        }
        println("functions=" + currentProgram.getFunctionManager().getFunctionCount());
        long codeBytes = 0;
        for (ghidra.program.model.listing.Instruction ins : currentProgram.getListing().getInstructions(true)) {
            codeBytes += ins.getLength();
        }
        println("instructions=" + currentProgram.getListing().getNumInstructions() + " codeBytes=" + codeBytes);
    }
}
