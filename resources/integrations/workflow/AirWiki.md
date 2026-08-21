# AirWiki

Use the `airwiki` skill whenever the user asks to create, select, consult, document, remember, or maintain knowledge with AirWiki.

AirWiki memory starts only after the user explicitly creates or selects a wiki for the current conversation or project. Once active, capture concise durable decisions, architecture, reusable procedures, confirmed conclusions, and known risks. List the wiki before writing and follow `nextCursor` as needed; before editing or deprecating a concept, read it with `wiki_id` and `concept_id`, omitting `cursor` and `limit`, and use its current Markdown body and latest fingerprint. Retry a write conflict once after reading again. After a timeout with an unknown outcome, inspect the wiki before deciding whether the mutation needs to be retried.

Never store secrets, credentials, personal data, private queries, logs, temporary state, speculation, or extensive file copies. Never verify, publish, share, grant access, or change permissions. If the user says "pause AirWiki" or "pausa AirWiki", stop automatic capture until explicitly resumed. If AirWiki is unavailable, continue the main task and report one pending synchronization without creating a replacement memory file.
