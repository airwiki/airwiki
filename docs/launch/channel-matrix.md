# Open-source channel matrix

This matrix ranks channels by fit and release maturity. It does not authorize
posting. A human owner must re-check each channel's live rules, create or use
the relevant account, and approve the final public text.

| Channel | Fit and timing | Official policy or entry point | Required message and preflight |
| --- | --- | --- | --- |
| GitHub repository topics and stable GitHub Release | First priority after stable release. It is the canonical source and download location. | [Topics](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/classifying-your-repository-with-topics) · [Releases](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository) | Set only accurate topics and a verified homepage. Publish the stable release only through the documented protected process. Include checksums, provenance, release notes, platform scope, known limits, support, security, and conduct routes. |
| Product Hunt | High discovery fit, **stable only**. Do not schedule or publish for an unsupported technical pre-release. | [Launch guidance](https://www.producthunt.com/launch) | Use [product-hunt.md](product-hunt.md), final gallery assets, verified stable URL and version, and staffed public replies. |
| Hacker News / Show HN | Useful for technical feedback. A clearly labelled usable beta may be appropriate; broad promotion should wait for stable. | [Show HN](https://news.ycombinator.com/showhn.html) · [Guidelines](https://news.ycombinator.com/newsguidelines.html) | Use [show-hn.md](show-hn.md). State beta trust limits exactly when applicable; never solicit votes or post duplicate announcements. |
| Curated Awesome lists | Good long-tail open-source discovery. Consider a local-first list for a beta and privacy-oriented lists after stable. Do not treat inclusion as guaranteed. | [GitHub contribution guidance](https://docs.github.com/en/get-started/exploring-projects-on-github/contributing-to-open-source) | Select an exact, maintained list whose scope fits AirWiki. Read that repository's `CONTRIBUTING.md` and PR template; propose one minimal factual entry only after the stable/beta status matches the list's rules. |
| Tauri community | Relevant developer audience after the desktop app's supported build and maintainer capacity are ready. | [Tauri Show and Tell](https://github.com/orgs/tauri-apps/discussions/categories/show-and-tell) | Re-check the category and current moderator/self-promotion rules immediately before posting. Lead with technical implementation and source; do not use the channel for end-user support or security reports. |
| Reddit communities | Optional, audience-specific follow-up after stable. Candidate communities may include open-source, privacy, local-first, and knowledge-management communities. | [Reddit content policy](https://www.redditinc.com/policies/content-policy) | Inspect the individual community rules and current self-promotion policy immediately before posting. Use a native, useful write-up rather than duplicating campaign copy; do not cross-post indiscriminately. |
| Peerlist and AlternativeTo | Optional directory discovery after stable; not launch-critical. | [Peerlist](https://peerlist.io/) · [AlternativeTo](https://alternativeto.net/) | Use only a verified stable URL, platform list, license, and feature descriptions. Re-check ownership, submission, and category requirements in the live interface. |
| Flathub, Homebrew, winget | Defer. These are package-distribution channels, not announcement sites, and need platform-specific support and signing/maintainer decisions. | [Flathub docs](https://docs.flathub.org/) · [Homebrew docs](https://docs.brew.sh/) · [winget docs](https://learn.microsoft.com/windows/package-manager/) | Do not submit until Linux desktop support or the relevant packaging plan exists, the exact platform artifact is supported, and ownership/update responsibility is staffed. |

## Channel sequence

1. Finish stable-release gates and make GitHub Release metadata, topics, support,
   security, conduct, and download facts accurate.
2. Publish the stable release through the protected release process.
3. Confirm that the launch owner can monitor public discussion, then choose
   Product Hunt and/or Show HN based on the release maturity.
4. Submit one curated-list proposal at a time, tailoring it to that list.
5. Use directories and community follow-ups only when they add a distinct,
   relevant audience.

Never fabricate a homepage, a funding model, review quotes, a launch date,
download counts, user counts, or compatibility status to meet a channel's
submission fields.
