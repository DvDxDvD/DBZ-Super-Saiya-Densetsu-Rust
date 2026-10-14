// Baseline: creates only the functions at the RESET/NMI/IRQ vectors (no trace), for comparison.
// @category DBZ
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;

public class DbzVectors extends GhidraScript {
    @Override
    public void run() throws Exception {
        long[] vec = { 0xFFFC, 0xFFEA, 0xFFEE };
        String[] names = { "RESET", "NMI", "IRQ" };
        for (int i = 0; i < vec.length; i++) {
            Address v = toAddr(vec[i]);
            long target = (getByte(v) & 0xFF) | ((getByte(v.add(1)) & 0xFF) << 8);
            Address t = toAddr(target);
            disassemble(t);
            createFunction(t, names[i]);
        }
        println("vector functions created");
    }
}
