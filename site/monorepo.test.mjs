import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { test } from "node:test";

const read = path => readFileSync(new URL(path, import.meta.url), "utf8");
const strataCrates = ["ferrite-strata", "ferrite-strata-cpu", "ferrite-strata-runtime", "ferrite-strata-plugin-api"];

test("repository and site use ferite while public crate names stay unchanged", () => {
  assert.match(read("../Cargo.toml"), /repository = "https:\/\/github\.com\/ostrium-labs\/ferite"/);
  assert.equal(JSON.parse(read("package.json")).name, "ferite-site");
  assert.equal(JSON.parse(read("package-lock.json")).name, "ferite-site");
  assert.equal(JSON.parse(read("wrangler.jsonc")).name, "ferite");
  assert.ok(read("app/components/brand/FerriteLogo.tsx").includes(">ferite</span>"));
});

test("Strata crates inherit the canonical workspace metadata and are locked", () => {
  const workspace = read("../Cargo.toml");
  const lockfile = read("../Cargo.lock");
  assert.match(workspace, /members = \["crates\/\*"\]/);
  for (const name of strataCrates) {
    assert.ok(workspace.includes(`${name} = { path = "crates/${name}"`));
    const manifest = read(`../crates/${name}/Cargo.toml`);
    assert.match(manifest, /repository\.workspace = true/);
    assert.match(manifest, /\[lints\]\s+workspace = true/);
    assert.ok(lockfile.includes(`name = "${name}"`));
    assert.ok(!existsSync(new URL(`../crates/${name}/.git`, import.meta.url)));
  }
});

test("shared navigation registers the Strata guides and their source files", () => {
  const navigation = read("app/content/navigation.ts");
  const registry = read("app/content/pages.ts");
  for (const slug of ["ouroboros", "strata", "strata-plugins"]) {
    assert.ok(navigation.includes(`slug: "${slug}"`));
    assert.ok(registry.includes(`slug: "${slug}"`));
    assert.ok(registry.includes(`../../content/docs/${slug}.mdx`));
    const document = read(`content/docs/${slug}.mdx`);
    assert.ok(document.length > 500);
    for (const match of document.matchAll(/\]\(\/docs\/([^/#)]+)(?:#[^)]*)?\)/g)) {
      assert.ok(navigation.includes(`slug: "${match[1]}"`), `unknown document ${match[1]}`);
    }
  }
  assert.match(read("react-router.config.ts"), /\.\.\.DOC_PATHS/);
  assert.match(read("app/content/paths.ts"), /DOC_NAV\.map/);
});

test("the source import preserves Strata decisions, notices and policy gates", () => {
  assert.ok(read("../NOTICE").includes("docs/strata/NOTICE"));
  assert.ok(read("../docs/strata/NOTICE").includes("Ferrite Strata"));
  assert.ok(read("../docs/strata/adr/0003-define-our-own-abi-rather-than-adopting-pjrt.md").length > 100);
  const workflow = read("../.github/workflows/ci.yml");
  for (const job of ["dependency-policy:", "cosim:", "synth:"]) assert.ok(workflow.includes(job));
});
