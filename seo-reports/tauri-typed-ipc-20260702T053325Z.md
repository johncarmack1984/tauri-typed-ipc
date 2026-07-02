# SEO/GEO audit - tauri-typed-ipc

`https://github.com/johncarmack1984/tauri-typed-ipc` | generated 20260702T053325Z | positioning: tauri-typed-ipc is a Rust crate (crates.io): trait-based, type-safe Tauri IPC built on specta v2, sync commands by default, with a generated drift-checked TypeScript client and a TauRPC codemod; no website of its own -- the surfaces are crates.io, docs.rs, lib.rs, and the GitHub repo.

## Findings (ranked)

- [HIGH] `crawl:schema.missing` - No JSON-LD structured data in server HTML (no Person/ProfilePage entity for Google or LLMs).
- [MED] `crawl:sitemap.missing` - No /sitemap.xml (200).
- [LOW] `crawl:title.length` - Title is 131 chars (aim 15-65).
- [LOW] `crawl:canonical.missing` - No rel=canonical link.
- [LOW] `crawl:robots.no_sitemap` - robots.txt does not reference a sitemap.
- [LOW] `gsc:gsc.no_property` - surface has no gsc_property set.
- [LOW] `bing:bing.error` - Bing API returned 400 (is the site verified for this key?).
- [LOW] `trends:trends.no_seeds` - surface has no seed_keywords.
- [LOW] `serper:serper.no_seeds` - surface has no seed_keywords; skipping.
- [LOW] `geo_probe:geo.no_config` - set surface_markers + geo_probes on the surface (surfaces.toml) to run GEO probes; skipping.

## Signals by provider

### crawl - Tier 0 (free) - ok
- **final_url**: https://github.com/johncarmack1984/tauri-typed-ipc
- **status**: 200
- **title**: GitHub - johncarmack1984/tauri-typed-ipc: Trait-based, type-safe Tauri IPC built on specta; sync by default, async opt-in. · GitHub
- **title_len**: 131
- **meta_description**: Trait-based, type-safe Tauri IPC built on specta; sync by default, async opt-in. - johncarmack1984/tauri-typed-ipc
- **canonical**: None
- **html_lang**: en
- **og_tags**: 9
- **twitter_tags**: 5
- **h1_raw**: Search code, repositories, users, issues, pull requests..., Provide feedback, Saved searches, johncarmack1984/tauri-typed-ipc, tauri-typed-ipc
- **raw_html_words**: 1257
- **jsonld_types_raw**: (none)
- **rendered**: skipped (install 'render' extra for the JS-gap)
- **robots_txt**: ok
- **sitemap_xml**: 406

### psi - Tier 0 (free) - error
- error: `HTTPStatusError: Server error '500 Internal Server Error' for url 'https://www.googleapis.com/pagespeedonline/v5/runPagespeed?url=https%3A%2F%2Fgithub.com%2Fjohncarmack1984%2Ftauri-typed-ipc&key=REDACTED&strategy=mobile&category=PERFORMANCE'
For more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/500`

### gsc - Tier 0 (free) - skipped

### github - Tier 0 (free) - ok
- **description**: Trait-based, type-safe Tauri IPC built on specta; sync by default, async opt-in.
- **topics**: codegen, ipc, rust, specta, tauri, type-safe, typescript
- **stars**: 0
- **homepage**: None
- **views_14d**: 2
- **uniques_14d**: 1

### bing - Tier 0 (free) - ok
- **bing**: HTTP 400

### trends - Tier 0 (free) - skipped

### dataforseo - Tier 1 (SERP/volume) - skipped

### serper - Tier 1 (SERP/volume) - skipped

### geo_probe - Tier 2 (GEO probes) - skipped

## Available if promoted (paid tiers, currently stubbed)

- **ahrefs** (Tier 3 (backlinks)): Backlink profile + referring domains + organic keyword/competitor gap - needs `AHREFS_API_KEY`
- **semrush** (Tier 3 (backlinks)): Domain/organic research + backlinks + competitor keyword gap (Semrush) - needs `SEMRUSH_API_KEY`
