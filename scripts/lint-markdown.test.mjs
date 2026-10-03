import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { formatDiagnostics, lintMarkdown, listMarkdownFiles } from "./lint-markdown.mjs";

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url));
const config = readFileSync(join(repositoryRoot, ".markdownlint.json"), "utf8");

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "loom-markdown-test-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  execFileSync("git", ["-c", "init.templateDir=", "init", "--quiet"], { cwd: root });
  writeFileSync(join(root, ".markdownlint.json"), config);
  return root;
}

function markdown(root, path, content = "# Fixture\n\nValid text.\n") {
  const destination = join(root, path);
  mkdirSync(dirname(destination), { recursive: true });
  writeFileSync(destination, content);
}

test("lints tracked and untracked Markdown but not ignored/generated/deleted files", (t) => {
  const root = fixture(t);
  markdown(root, "tracked.md");
  markdown(root, "deleted.md");
  execFileSync("git", ["add", "tracked.md", "deleted.md"], { cwd: root });
  rmSync(join(root, "deleted.md"));
  markdown(root, "docs/untracked.md");
  writeFileSync(join(root, ".gitignore"), "ignored/\n");
  markdown(root, "ignored/bad.md", "not lintable");
  for (const directory of ["target", "node_modules", "dist", "build", "coverage", ".forge", "src-tauri/gen"]) {
    markdown(root, `${directory}/bad.md`, "not lintable");
  }
  assert.deepEqual(listMarkdownFiles(root), ["docs/untracked.md", "tracked.md"]);
  assert.equal(lintMarkdown(root).violations, 0);
});

test("braces, spaces, and newlines in filenames remain literal paths", (t) => {
  const root = fixture(t);
  const names = ["docs/{a,b}.md", "docs/a b.md", "docs/line\nbreak.md"];
  for (const name of names) markdown(root, name);
  assert.deepEqual(listMarkdownFiles(root), names.sort());
  assert.equal(lintMarkdown(root).violations, 0);
});

test("invalid Markdown produces inspectable rule violations", (t) => {
  const root = fixture(t);
  markdown(root, "bad.md", "not a heading\n");
  const report = lintMarkdown(root);
  assert.equal(report.violations, 1);
  assert.ok(report.results["bad.md"][0].ruleNames.includes("MD041"));
  assert.match(formatDiagnostics(report.results), /"bad\.md":1:1 MD041/);
});

test("preserves the existing heading, HTML, code, table, and line-length policy", (t) => {
  const root = fixture(t);
  const long = "word ".repeat(25).trim();
  markdown(root, "valid.md", `# Fixture\n\n<span>Allowed HTML</span>\n\n\`\`\`text\n${long}\n\`\`\`\n\n| Value |\n| --- |\n| ${long} |\n`);
  assert.equal(lintMarkdown(root).violations, 0);
  markdown(root, "long.md", `# Fixture\n\n${long}\n`);
  const report = lintMarkdown(root);
  assert.equal(report.violations, 1);
  assert.ok(report.results["long.md"][0].ruleNames.includes("MD013"));
});

test("refuses Markdown symlinks instead of reading outside the selected inventory", (t) => {
  const root = fixture(t);
  markdown(root, "original.md");
  symlinkSync("original.md", join(root, "linked.md"));
  assert.throws(() => lintMarkdown(root), /not a regular file.*linked\.md/);
});

test("the lint toolchain has no vulnerable brace/glob wrapper dependency", () => {
  const manifest = JSON.parse(readFileSync(join(repositoryRoot, "package.json"), "utf8"));
  const lock = JSON.parse(readFileSync(join(repositoryRoot, "package-lock.json"), "utf8"));
  assert.equal(manifest.devDependencies.markdownlint, "0.41.1");
  assert.equal(manifest.devDependencies["markdownlint-cli2"], undefined);
  for (const dependency of ["markdownlint-cli2", "globby", "fast-glob", "micromatch", "braces"]) {
    assert.equal(lock.packages[`node_modules/${dependency}`], undefined, dependency);
  }
});
