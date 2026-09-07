/*
 * Release switch — edit this file only after the stable-release GO has been
 * approved and the signed, notarized installers have passed their acceptance
 * checks. Keep the technical-beta values until then.
 */
window.AIRWIKI_LAUNCH = {
  releaseState: "technical-beta",
  primaryCta: {
    label: "View source",
    href: "https://github.com/airwiki/airwiki",
  },
  secondaryCta: {
    label: "Technical beta for evaluators",
    href: "https://github.com/airwiki/airwiki/releases",
    notice:
      "Development candidates only. Windows packages remain unsigned; check each release for macOS signing status. Keep platform protections enabled.",
  },
};
