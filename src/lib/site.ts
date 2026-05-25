function normalizeBasePath(value: string | undefined): string {
  const trimmed = value?.trim() ?? "";

  if (!trimmed || trimmed === "/") {
    return "";
  }

  return `/${trimmed.replace(/^\/+|\/+$/g, "")}`;
}

function normalizeSiteUrl(value: string | undefined): string {
  const trimmed = value?.trim().replace(/\/+$/, "") ?? "";

  if (trimmed) {
    return trimmed;
  }

  return "https://gryszzz.github.io/OpenThymos";
}

const basePath = normalizeBasePath(process.env.NEXT_PUBLIC_BASE_PATH);
const githubUrl = "https://github.com/gryszzz/OpenThymos";

export const siteConfig = {
  name: process.env.NEXT_PUBLIC_APP_NAME || "OpenThymos",
  tagline: "Execution substrate for governed machine cognition.",
  headline: "OpenThymos",
  subheadline:
    "A Rust runtime that turns model output into typed intents, checks them against signed authority, executes approved tools, and records every outcome in a replayable ledger.",
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
