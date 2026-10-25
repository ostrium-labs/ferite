/** Lightweight navigation metadata. No MDX or long-form content enters the site header. */
export type DocSection = "background" | "crates" | "corpus" | "benchmarks" | "strata";
export interface DocNavigation {
  slug: string;
  label: string;
  section: DocSection;
  prerequisite?: string;
}

export const DOC_NAV: DocNavigation[] = [
  { slug: "ouroboros", label: "Lithic ↔ Strata", section: "strata" },
  { slug: "strata", label: "Strata runtime", section: "strata", prerequisite: "ouroboros" },
  { slug: "strata-plugins", label: "Backend & plugin boundary", section: "strata", prerequisite: "strata" },
  { slug: "basics", label: "Circuit basics", section: "background" },
  { slug: "ocaml", label: "OCaml & hardware DSLs", section: "background", prerequisite: "basics" },
  { slug: "hardcaml", label: "The Hardcaml model", section: "background", prerequisite: "ocaml" },
  { slug: "why-rust", label: "Why Rust", section: "background", prerequisite: "hardcaml" },
  { slug: "bits", label: "Bits", section: "crates", prerequisite: "basics" },
  { slug: "ir", label: "IR / the graph", section: "crates", prerequisite: "bits" },
  { slug: "design", label: "Design / Rust API", section: "crates", prerequisite: "ir" },
  { slug: "sim", label: "Cycle simulator", section: "crates", prerequisite: "design" },
  { slug: "rtl", label: "Verilog emitter", section: "crates", prerequisite: "ir" },
  { slug: "derive", label: "Ports by derive", section: "crates", prerequisite: "design" },
  { slug: "wave", label: "Waveform data", section: "crates", prerequisite: "sim" },
  { slug: "tb", label: "Step testbenches", section: "crates", prerequisite: "sim" },
  { slug: "cosim", label: "Verilator equivalence", section: "crates", prerequisite: "rtl" },
  { slug: "pdk", label: "The process", section: "crates", prerequisite: "rtl" },
  { slug: "chip", label: "The submission boundary", section: "crates", prerequisite: "pdk" },
  { slug: "corpus", label: "The corpus", section: "corpus", prerequisite: "cosim" },
  { slug: "findings", label: "Bugs & findings", section: "corpus", prerequisite: "corpus" },
  { slug: "benchmarks", label: "Measured benchmarks", section: "benchmarks", prerequisite: "corpus" },
];

export const DOC_GROUPS: { id: string; title: string; sections: DocSection[]; description: string }[] = [
  { id: "strata", title: "Strata & the feedback loop", sections: ["strata"], description: "Workloads feed hardware; verified hardware feeds runtime capabilities. Follow the shared contract and its current limits." },
  { id: "concepts", title: "Concepts", sections: ["background"], description: "Learn the circuit model and the choices inherited from Hardcaml." },
  { id: "toolchain", title: "Toolchain", sections: ["crates"], description: "Follow a value from bits to the graph, then into simulation and RTL." },
  { id: "evidence", title: "Verification & evidence", sections: ["corpus", "benchmarks"], description: "Read the external checks, the defects they exposed and the measured costs." },
];

export const DOC_COUNT = DOC_NAV.length;
export const SOURCE_ROOT = "https://github.com/ostrium-labs/ferite/blob/dev/";
