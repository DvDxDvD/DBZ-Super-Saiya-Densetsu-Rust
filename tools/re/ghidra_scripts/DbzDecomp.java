// Diagnostic: prints the decompiled C of the functions at the given addresses (e.g. "80eb,8000").
// @category DBZ
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileOptions;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.Function;

public class DbzDecomp extends GhidraScript {
    @Override
    public void run() throws Exception {
        DecompInterface di = new DecompInterface();
        DecompileOptions opts = new DecompileOptions();
        opts.grabFromProgram(currentProgram);
        println("respectReadOnly=" + opts.isRespectReadOnly());
        opts.setRespectReadOnly(true);
        di.setOptions(opts);
        di.openProgram(currentProgram);
        for (String s : getScriptArgs()[0].split(",")) {
            Function f = getFunctionContaining(toAddr(Long.parseLong(s, 16)));
            if (f == null) {
                println("no function at " + s);
                continue;
            }
            DecompileResults r = di.decompileFunction(f, 60, monitor);
            println(r.getDecompiledFunction() != null ? r.getDecompiledFunction().getC() : r.getErrorMessage());
        }
        di.dispose();
    }
}
