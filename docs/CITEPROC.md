# Citation processor (`superdocuments::citeproc`)

A CSL 1.0.x processor with Pandoc's citation syntax, for author-date, numeric and note styles. Local only: the
style is the caller's, the English terms are built in, nothing is fetched and no process is started.

```rust
use superdocuments::citeproc::{self, markdown, Processor, Style};

let refs = citeproc::references_from_json(&csl_json)?;          // CSL-JSON, or bibtex::csl_json_pandoc(..)
let p = Processor::new(Style::parse(&csl_xml)?, refs);
let md = markdown::process_markdown("As shown [@a, p. 4; @b].", &p); // citations replaced, `refs` div appended
```

`markdown::parse_citations` reads `[@a; @b, p. 4]`, `[see @a, ch. 3]`, `-@a` (suppress author), `@a [p. 3]`
(author in text), skipping code and the YAML header; `Processor::process` takes `Citation`/`CitationItem` values
directly (id, prefix, suffix, label, locator, suppress-author, author-in-text) and returns token lists
(`to_inlines` gives the shared inline tree, `comparison_string` a string for exact comparison).

**Implemented:** `text`, `number`, `label`, `date` (ranges, seasons, locale forms, `date-part` overrides, AD/BC),
`names` (et al. rules, initials, particles, `and`, labels, `substitute`, `names-min` overrides from
disambiguation), `group` (implicit suppression, `choose` branches count as group items), `choose` (`type`, `variable`,
`is-numeric`, `is-uncertain-date`, `locator`, `position`, `disambiguate`), macros, sorting for citations and bibliography,
`disambiguate-add-names`, conditional disambiguation, `disambiguate-add-year-suffix`, `collapse` (year, year-suffix,
citation-number) with grouping of cites by author, `subsequent-author-substitute`, `second-field-align`, text cases,
quotes with American punctuation, affixes, font attributes, `page-range-format` and `page-range-delimiter`, style
`<locale>` term overrides (English).

**Not implemented:** `disambiguate-add-givenname`, `year-suffix-ranged`, locale files other than English, `ibid`
positions beyond first/subsequent, `cite-group-delimiter` corner cases, hyperlinks (DOI/URL text is printed, not linked).

**Pandoc names:** `Processor::with_pandoc_names(true)` reads `<names>` without `<name>` as Pandoc does (ignoring
style-level name options). Off by default.

`bibtex::csl_json_pandoc(&bib, biblatex)` converts BibTeX/BibLaTeX the way Pandoc does before formatting (sentence-case
titles, `publisher` from `howpublished`/`school`/`institution`, thesis `genre`, `number` as `issue` for articles,
dropping particles); `bibtex::csl_json` is the plain export and is unchanged.

Agreement with `pandoc --citeproc`, the benchmark and how to reproduce it: `benches/pandoc-compat/README.md`.
Remaining differences and their causes: `benches/pandoc-compat/DISAGREEMENTS.md`.
