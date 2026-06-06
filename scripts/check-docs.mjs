import fs from "node:fs";
import path from "node:path";

const repoRoot = process.cwd();
const docsRoots = ["README.md", "thymos/README.md", "docs", "wiki"];
const forbiddenRepoUrls = ["https://github.com/gryszzz/THYMOS"];
const forbiddenCustomDomains = [`thymos${"."}ai`];

function walk(targetPath) {
  const absolutePath = path.join(repoRoot, targetPath);
  if (!fs.existsSync(absolutePath)) {
    return [];
  }

  const stat = fs.statSync(absolutePath);
  if (stat.isFile()) {
    return [absolutePath];
  }

  return fs.readdirSync(absolutePath, { withFileTypes: true }).flatMap((entry) => {
    const nextPath = path.join(targetPath, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "_site" || entry.name === "node_modules") {
        return [];
      }
      return walk(nextPath);
    }
    return [path.join(repoRoot, nextPath)];
  });
}

function slugifyHeading(text) {
  return text
    .trim()
    .toLowerCase()
    .replace(/[`*_~]/g, "")
    .replace(/[^\w\s-]/g, "")
    .replace(/\s+/g, "-");
}

function collectAnchors(filePath) {
  const content = fs.readFileSync(filePath, "utf8");
  const anchors = new Set([""]);

  for (const match of content.matchAll(/^#{1,6}\s+(.+)$/gm)) {
    anchors.add(slugifyHeading(match[1]));
  }

  for (const match of content.matchAll(/<a\s+id=["']([^"']+)["']/g)) {
    anchors.add(match[1]);
  }

  return anchors;
}

function resolveLink(filePath, rawTarget) {
  const liquidMatch = rawTarget.match(/^\{\{\s*'([^']+)'\s*\|\s*relative_url\s*\}\}$/);
  const normalizedTarget = liquidMatch ? liquidMatch[1] : rawTarget;
  const [targetPath, anchor = ""] = normalizedTarget.split("#");
  if (!targetPath) {
    return { kind: "anchor", anchor };
  }

  if (liquidMatch) {
    const docsPath = targetPath === "/" ? "index.md" : `${targetPath.replace(/^\//, "")}.md`;
    return {
      kind: "file",
      resolvedPath: path.join(repoRoot, "docs", docsPath),
      anchor,
    };
  }

  return {
    kind: "file",
    resolvedPath: path.resolve(path.dirname(filePath), targetPath),
    anchor,
  };
}

function isExternal(target) {
  return (
    target.startsWith("http://") ||
    target.startsWith("https://") ||
    target.startsWith("mailto:") ||
    target.startsWith("tel:")
  );
}

function isIgnored(target) {
  return (
    target.startsWith("app://") ||
    target.startsWith("plugin://") ||
    target.startsWith("vscode://")
  );
}

const markdownFiles = docsRoots
  .flatMap((entry) => walk(entry))
  .filter((filePath) => /\.(md|html)$/i.test(filePath))
  .sort();

const errors = [];

for (const filePath of markdownFiles) {
  const content = fs.readFileSync(filePath, "utf8");
  const relativePath = path.relative(repoRoot, filePath);

  for (const forbiddenUrl of forbiddenRepoUrls) {
    if (content.includes(forbiddenUrl)) {
      errors.push(`${relativePath}: stale repo URL ${forbiddenUrl}`);
    }
  }

  for (const customDomain of forbiddenCustomDomains) {
    if (content.includes(customDomain)) {
      errors.push(`${relativePath}: stale custom domain ${customDomain}`);
    }
  }

  for (const match of content.matchAll(/\[[^\]]*\]\(([^)]+)\)/g)) {
    const target = match[1].trim();
    if (!target || isExternal(target) || isIgnored(target)) {
      continue;
    }

    const resolved = resolveLink(filePath, target);
    if (resolved.kind === "anchor") {
      const anchors = collectAnchors(filePath);
      if (!anchors.has(resolved.anchor)) {
        errors.push(`${relativePath}: missing anchor #${resolved.anchor}`);
      }
      continue;
    }

    if (!fs.existsSync(resolved.resolvedPath)) {
      errors.push(
        `${relativePath}: missing target ${path.relative(repoRoot, resolved.resolvedPath)}`,
      );
      continue;
    }

    if (resolved.anchor) {
      // Heading anchors only exist in prose. For source files, `#L99` /
      // `#L99-L120` are GitHub line anchors (valid on the web, no heading to
      // match), so accept those without trying to resolve a heading.
      const isProseTarget = /\.(md|html)$/i.test(resolved.resolvedPath);
      const isLineAnchor = /^L\d+(-L\d+)?$/.test(resolved.anchor);
      if (isProseTarget && !isLineAnchor) {
        const anchors = collectAnchors(resolved.resolvedPath);
        if (!anchors.has(resolved.anchor)) {
          errors.push(
            `${relativePath}: missing anchor #${resolved.anchor} in ${path.relative(repoRoot, resolved.resolvedPath)}`,
          );
        }
      }
    }
  }
}

if (errors.length > 0) {
  console.error("Documentation check failed:\n");
  for (const error of errors) {
    console.error(`- ${error}`);
  }
  process.exit(1);
}

console.log(`Documentation check passed for ${markdownFiles.length} files.`);
