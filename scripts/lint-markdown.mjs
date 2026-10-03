import { execFileSync } from "node:child_process";
import { lstatSync, readFileSync } from "node:fs";
import { isAbsolute, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { lint } from "markdownlint/sync";

const excludedParts = new Set([
  ".git", ".forge", "node_modules", "target", "dist", "build", "coverage",
]);
const repositoryRoot = fileURLToPath(new URL("..", import.meta.url));

/** Git supplies literal, NUL-delimited paths. No filename is interpreted as a glob. */
export function listMarkdownFiles(root = repositoryRoot) {
  const absoluteRoot = resolve(root);
  const inventory = execFileSync(
    "git", ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    { cwd: absoluteRoot, encoding: "utf8", maxBuffer: 8 * 1024 * 1024 },
  );
  const paths = [...new Set(inventory.split("\0").filter((path) => path.endsWith(".md")))];
  return paths.sort().filter((path) => {
    const parts = path.split("/");
    if (parts.some((part) => excludedParts.has(part))) return false;
    if (path.startsWith("src-tauri/gen/")) return false;
    const absolutePath = resolve(absoluteRoot, path);
    if (isAbsolute(path) || !absolutePath.startsWith(`${absoluteRoot}${sep}`)) {
      throw new Error(`Markdown path escapes the repository: ${JSON.stringify(path)}`);
    }
    let stat;
    try {
      stat = lstatSync(absolutePath);
    } catch (error) {
      if (error.code === "ENOENT") return false; // A tracked file may be deleted locally.
      throw error;
    }
    if (!stat.isFile() || stat.isSymbolicLink()) {
      throw new Error(`Markdown path is not a regular file: ${JSON.stringify(path)}`);
    }
    return true;
  });
}

export function lintMarkdown(root = repositoryRoot) {
  const absoluteRoot = resolve(root);
  const files = listMarkdownFiles(absoluteRoot);
  const config = JSON.parse(readFileSync(resolve(absoluteRoot, ".markdownlint.json"), "utf8"));
  const strings = Object.create(null);
  for (const path of files) strings[path] = readFileSync(resolve(absoluteRoot, path), "utf8");
  const results = lint({ strings, config });
  const violations = Object.values(results).reduce((total, errors) => total + errors.length, 0);
  return { files, results, violations };
}

export function formatDiagnostics(results) {
  return Object.entries(results).flatMap(([path, errors]) => errors.map((error) => {
    const column = error.errorRange?.[0] ?? 1;
    const detail = error.errorDetail ? ` (${error.errorDetail})` : "";
    return `${JSON.stringify(path)}:${error.lineNumber}:${column} ${error.ruleNames[0]} ${error.ruleDescription}${detail}`;
  })).join("\n");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const { files, results, violations } = lintMarkdown();
    if (violations > 0) process.stderr.write(`${formatDiagnostics(results)}\n`);
    process.stdout.write(`Markdown: ${files.length} files, ${violations} violations\n`);
    process.exitCode = violations > 0 ? 1 : 0;
  } catch (error) {
    process.stderr.write(`Markdown lint failed: ${error.message}\n`);
    process.exitCode = 1;
  }
}
