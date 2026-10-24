# GML 3.2.1 schemas (vendored, unmodified)

These are the OGC schema files for GML 3.2.1 (ISO 19136:2007), as published under
`https://schemas.opengis.net/gml/3.2.1/`, with the ISO 19139 (`gmd`, `gco`, `gss`, `gsr`, `gts`) files
that `gml.xsd` imports and the W3C `xlink.xsd` and `xml.xsd` (the `xml:` attributes `xlink` uses). The files in this directory are byte-for-byte what the
servers returned; `SHA256SUMS` lists their digests and the GML export's test suite refuses to run if one
has changed.

I vendor them because the validation container has no network, and because a test that fetches its
schema at run time validates against whatever the server says that day. The files carry the OGC
copyright notice and are distributed under the OGC document licence
(<http://www.opengeospatial.org/legal/>), which permits copying them unmodified.

The schema documents inside `gml/3.2.1/` say `version="3.2.2"`: OGC published the editorial revisions of
GML 3.2.1 under the same directory and the same namespace (`http://www.opengis.net/gml/3.2`). That is the
"declared GML version" of `docs/INTERCHANGE.md` chapter 3.

Some `schemaLocation` values are absolute URLs (`xlink.xsd` and `xml.xsd` at w3.org, and the `gmd` tree at
`schemas.opengis.net`). The validator (`crates/pvec-interchange/tests/support/Validate.java`) maps them
onto these local files with a resolver, so the schema documents stay unmodified.
