# Open-source channel matrix

This matrix ranks channels by fit and release maturity. It does not authorize
posting. A human owner must re-check each channel's live rules, create or use
the relevant account, and approve the final public text.

| Channel | Fit and timing | Official policy or entry point | Required message and preflight |
| --- | --- | --- | --- |
| GitHub repository topics and stable GitHub Release | First priority after stable release. It is the canonical source and download location. | [Topics](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/classifying-your-repository-with-topics) · [Releases](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository) | Set only accurate topics and a verified homepage. Publish the stable release only through the documented protected process. Include checksums, provenance, release notes, platform scope, known limits, support, security, and conduct routes. |
| DEV Community | Good pre-stable channel for a substantial technical article and narrowly scoped feedback. It is not an announcement mirror. | [New post](https://dev.to/new) · [content policy](https://dev.to/terms) · [writing help](https://dev.to/help/writing-editing-scheduling) | Publish useful, self-contained technical material: architecture, privacy boundaries, reproducible beta evaluation, and known limits. A post must not be designed primarily for promotion or backlinks. Re-check current AI-assistance, tagging, canonical-URL, and submission rules before sending. |
| Indie Hackers | Good pre-stable channel for a founder/build-in-public discussion or a request for feedback from builders. It is secondary to the technical community. | [New post](https://www.indiehackers.com/new-post) · [official posting guidance](https://www.indiehackers.com/post/how-do-you-make-a-successful-post-on-indie-hackers-f6745260fd) | Lead with a useful lesson or a concrete feedback question, not a launch announcement. The post should stand on its own; put a relevant beta/source link at the end. New accounts can be subject to anti-spam controls, so participate genuinely and re-check the live posting rules rather than manufacturing engagement. |
| Product Hunt | High discovery fit, **stable only**. Do not schedule or publish an unsupported technical pre-release. | [Submission](https://www.producthunt.com/posts/new) · [launch guidance](https://www.producthunt.com/launch) · [posting requirements](https://help.producthunt.com/en/articles/479557-how-to-post-a-product) | Use [product-hunt.md](product-hunt.md), final gallery assets, verified stable URL and version, and staffed public replies. The maker must use an eligible personal account. Never pay a hunter, buy promotion, or ask anyone to upvote; invite honest feedback only. |
| Hacker News / Show HN | Useful for technical feedback. A clearly labelled, self-service usable beta may be appropriate; broad promotion should wait for stable. | [Submit](https://news.ycombinator.com/submit) · [Show HN](https://news.ycombinator.com/showhn.html) · [guidelines](https://news.ycombinator.com/newsguidelines.html) | Use [show-hn.md](show-hn.md). State beta trust limits exactly when applicable; never solicit votes, comments, or coordinated traffic, and do not repost a prior announcement. A human who built the project must be available for the discussion; re-check current account eligibility before posting. |
| Curated Awesome lists | Good long-tail open-source discovery. Consider a local-first list for a beta and privacy-oriented lists after stable. Do not treat inclusion as guaranteed. | [GitHub contribution guidance](https://docs.github.com/en/get-started/exploring-projects-on-github/contributing-to-open-source) | Select an exact, maintained list whose scope fits AirWiki. Read that repository's `CONTRIBUTING.md` and PR template; propose one minimal factual entry only after the stable/beta status matches the list's rules. |
| Tauri community | Relevant developer audience after the desktop app's supported build and maintainer capacity are ready. | [Tauri Show and Tell](https://github.com/orgs/tauri-apps/discussions/categories/show-and-tell) | Re-check the category and current moderator/self-promotion rules immediately before posting. Lead with technical implementation and source; do not use the channel for end-user support or security reports. |
| Reddit communities | Optional, audience-specific follow-up after stable. Candidate communities may include open-source, privacy, local-first, and knowledge-management communities. | [Reddit content policy](https://www.redditinc.com/policies/content-policy) | Inspect the individual community rules and current self-promotion policy immediately before posting. Use a native, useful write-up rather than duplicating campaign copy; do not cross-post indiscriminately. |
| AlternativeTo | Useful long-tail directory discovery after a stable release; not launch-critical. | [New-app instructions](https://alternativeto.net/faq/#add-a-new-application) | Submit only an accurate, English-supported stable listing with official URL, platforms, license, description, tags, and screenshots. Email verification and editorial review apply. Do not use a personal profile to advertise, and do not pay for priority review without the owner's explicit approval. |
| Peerlist Launchpad | Optional product-community discovery after stable; less specific to open source than AlternativeTo. | [Launchpad rules](https://help.peerlist.io/individual/launchpad/guidelines-faqs) | Use an eligible individual profile and re-check the live submission flow and eligibility criteria immediately before sending. Prepare a factual tagline, logo/screenshots or demo, stable URL, license, platform scope, and known limits. Peerlist prohibits unsolicited upvote requests and repeated launch-link sharing; do not purchase a boost or coordinate votes without explicit owner approval. |
| OpenSourceAlternative.to | **Conditional; do not submit yet.** It fits only if AirWiki has a supported self-hosted offering and is a genuine alternative to a named proprietary product. Local-first desktop use alone does not establish that fit. | [Submission criteria](https://opensourcealternative.to/submit) | Revalidate that the project is open source, actively maintained, self-hosted, and a genuine alternative at the time of submission. Do not pay for expedited review or submit an unsupported self-hosting claim without explicit owner approval. |
| Flathub, Homebrew, winget | Defer. These are package-distribution channels, not announcement sites, and need platform-specific support and signing/maintainer decisions. | [Flathub docs](https://docs.flathub.org/) · [Homebrew docs](https://docs.brew.sh/) · [winget docs](https://learn.microsoft.com/windows/package-manager/) | Do not submit until Linux desktop support or the relevant packaging plan exists, the exact platform artifact is supported, and ownership/update responsibility is staffed. |

## Channel sequence

1. Before stable release, prepare a substantive DEV article and an Indie Hackers
   feedback discussion. They must disclose the beta scope and provide value even
   if a reader never follows a link.
2. If a public, self-service technical beta is actually usable, consider one
   honest Show HN discussion. Do not substitute an invitation-only preview,
   source checkout, or landing page for something readers can try.
3. Use the technical feedback to correct documentation, onboarding, and known
   limits. Do not make stable-release claims, collect vanity metrics, or run a
   vote campaign.
4. Finish stable-release gates and make GitHub Release metadata, topics, support,
   security, conduct, privacy, and download facts accurate. Publish only through
   the protected release process.
5. Confirm that the launch owner can monitor public discussion, then consider
   Product Hunt, AlternativeTo, and Peerlist independently. Submit one
   directory/listing at a time, tailored to that channel's live rules.

Never fabricate a homepage, a funding model, review quotes, a launch date,
download counts, user counts, or compatibility status to meet a channel's
submission fields. Never link-dump, cross-post campaign text indiscriminately,
coordinate votes/comments, use sockpuppets, or offer incentives for engagement.
Every channel's requirements, eligibility, payment options, and asset fields
must be revalidated in its live interface immediately before a human submits;
this document deliberately does not preserve changing thresholds or metrics.
