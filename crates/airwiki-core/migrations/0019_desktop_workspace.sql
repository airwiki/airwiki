-- One bounded, local presentation preference. No query, text, path or remote identity.
CREATE TABLE desktop_workspace (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    version INTEGER NOT NULL CHECK (version = 1),
    wiki_id TEXT CHECK (wiki_id IS NULL OR length(wiki_id) = 36),
    page_kind TEXT CHECK (page_kind IN ('index', 'log', 'concept')),
    concept_id TEXT CHECK (concept_id IS NULL OR length(concept_id) = 36),
    sidebar_width INTEGER NOT NULL CHECK (sidebar_width BETWEEN 200 AND 360),
    sidebar_collapsed INTEGER NOT NULL CHECK (sidebar_collapsed IN (0, 1)),
    CHECK (
        (wiki_id IS NULL AND page_kind IS NULL AND concept_id IS NULL)
        OR (wiki_id IS NOT NULL AND page_kind IS NOT NULL AND
            ((page_kind = 'concept' AND concept_id IS NOT NULL)
             OR (page_kind IN ('index', 'log') AND concept_id IS NULL)))
    )
);
