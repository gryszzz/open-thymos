function normalizeBasePath(value: string | undefined): string {
  const trimmed = value?.trim() ?? "";

  if (!trimmed || trimmed === "/") {
    return "";
  }

  return `/${trimmed.replace(/^\/+|\/+$/g, "")}`;
}

const defaultSiteUrl = "https://gryszzz.github.io/open-thymos";
const allowedSiteHosts = new Set(["gryszzz.github.io", "localhost", "127.0.0.1", "::1"]);

function normalizeSiteUrl(value: string | undefined): string {
  const trimmed = value?.trim().replace(/\/+$/, "") ?? "";

  if (!trimmed) {
    return defaultSiteUrl;
  }

  try {
    const url = new URL(trimmed);
    const hostname = url.hostname.toLowerCase().replace(/^\[|\]$/g, "");

    if ((url.protocol === "http:" || url.protocol === "https:") && allowedSiteHosts.has(hostname)) {
      return url.toString().replace(/\/+$/, "");
    }
  } catch {
    return defaultSiteUrl;
  }

  return defaultSiteUrl;
}

const basePath = normalizeBasePath(process.env.NEXT_PUBLIC_BASE_PATH);
const githubUrl = "https://github.com/gryszzz/open-thymos";

export const siteConfig = {
  name: process.env.NEXT_PUBLIC_APP_NAME || "OpenThymos",
  tagline: "Unified AI execution runtime, framework, and sandbox for coding agents.",
  headline: "OpenThymos",
  subheadline:
    "A Rust framework that turns model output into typed intents, checks them against signed authority, routes approved capabilities through governed execution boundaries, and exposes one replayable state across CLI, VS Code, terminal, and web surfaces.",
  basePath,
  siteUrl: normalizeSiteUrl(process.env.NEXT_PUBLIC_SITE_URL),
  githubUrl,
  docsUrl: `${githubUrl}/tree/main/docs`,
  packageDocsUrl: `${githubUrl}/blob/main/docs/package-distribution.md`,
  packagesUrl: `${githubUrl}/pkgs/container/openthymos-runtime`,
  issuesUrl: `${githubUrl}/issues`,
  readmeUrl: `${githubUrl}#readme`,
  wikiUrl: `${githubUrl}/wiki`,
  org: "Exponet Labs",
};
