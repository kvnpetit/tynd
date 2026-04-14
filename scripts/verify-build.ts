import { readFileSync } from "fs";
import { NtExecutable, NtExecutableResource, Resource } from "resedit";
import path from "path";

function checkExe(filePath: string) {
  console.log(`\nInspecting: ${filePath}`);
  try {
    const buffer = readFileSync(filePath);
    const exe = NtExecutable.from(buffer);
    
    // 1. Check Subsystem
    const subsystem = exe.optionalHeader.subsystem;
    const subsystemName = subsystem === 2 ? "WINDOWS_GUI" : subsystem === 3 ? "WINDOWS_CUI (Console)" : "UNKNOWN (" + subsystem + ")";
    console.log(`  - Subsystem: ${subsystemName}`);

    // 2. Check Resources
    const res = NtExecutableResource.from(exe);
    const iconGroups = res.entries.filter(e => e.type === Resource.Type.IconGroup);
    console.log(`  - Icon Groups found: ${iconGroups.length}`);
    
    if (iconGroups.length > 0) {
      console.log(`  - Icon IDs: ${iconGroups.map(g => g.id).join(", ")}`);
    }

    if (subsystem === 2 && iconGroups.length > 0) {
      console.log(`  ✅ VALID: GUI + Icon present`);
    } else {
      console.log(`  ❌ INVALID: Missing GUI subsystem or Icon`);
    }
  } catch (e) {
    console.log(`  ❌ ERROR: ${e.message}`);
  }
}

const releaseDir = path.join(process.cwd(), "examples/hello-world/release");
checkExe(path.join(releaseDir, "hello-world.exe"));
checkExe(path.join(releaseDir, "hello-world-portable.exe"));
checkExe(path.join(releaseDir, "hello-world-setup.exe"));
