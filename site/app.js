(function configureLaunchCtas() {
  "use strict";

  const launch = window.AIRWIKI_LAUNCH;
  if (!launch) return;

  document.documentElement.dataset.releaseState = launch.releaseState;

  for (const cta of document.querySelectorAll("[data-primary-cta]")) {
    cta.textContent = launch.primaryCta.label;
    cta.href = launch.primaryCta.href;
  }

  for (const cta of document.querySelectorAll("[data-secondary-cta]")) {
    cta.textContent = launch.secondaryCta.label;
    cta.href = launch.secondaryCta.href;
  }

  for (const notice of document.querySelectorAll("[data-release-notice]")) {
    notice.textContent = launch.secondaryCta.notice;
  }
})();
