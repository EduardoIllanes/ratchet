# pdf

Local text extraction from a PDF already on disk, via the external `liteparse` CLI. There is no
network access anywhere in this capability.

## Purpose

A PDF is opaque to an agent until its text is out where the agent can read it in bounded slices.
`ratchet pdf` does exactly that and nothing else: it validates the file, runs the external
extractor with an automatic OCR retry for scanned pages, writes the result to a sink file with a
predictable name, and shows the terminal a header — never the body.

## Requirements

### Requirement: Input must be an existing, readable PDF file
A missing path, or a file whose first bytes are not the PDF magic number, SHALL be refused
before any extractor is invoked, naming the file.

#### Scenario: Missing file is refused
- **WHEN** `ratchet pdf` is run on a path that does not exist
- **THEN** the call fails naming the file as not found, and no extractor is invoked

#### Scenario: Non-PDF file is refused
- **WHEN** `ratchet pdf` is run on a file that exists but does not start with the PDF magic bytes
- **THEN** the call fails saying the file is not a PDF, and no extractor is invoked

### Requirement: A missing extractor refuses the call before any work
The external extractor named in the configuration SHALL be resolved once, before the input file
is read for extraction. When it cannot be found, the call SHALL fail with the install command in
the message, and no extraction SHALL be attempted.

#### Scenario: Missing extractor refuses before any extraction
- **WHEN** the configured extractor cannot be found
- **THEN** the call fails with the install command in the message, and the extractor is never
  invoked

### Requirement: A page range narrows the extraction and names the sink file
`--pages` SHALL be forwarded to the extractor unchanged, and SHALL be part of the sink file's
name, so two different ranges of the same document never collide on disk.

#### Scenario: Page range narrows the extraction and the sink file name
- **WHEN** `ratchet pdf` is run with `--pages "2-3"`
- **THEN** the extractor receives that range, and the sink file's name carries it

### Requirement: Automatic OCR retry on nearly empty text
When the fast pass's text falls below the configured minimum, the extractor SHALL be invoked a
second time with OCR, and that text SHALL be used instead — declared in the header as a
fallback. `--ocr` SHALL skip the fast pass entirely and go straight to OCR, declared as forced.

#### Scenario: Nearly empty text retries with OCR
- **WHEN** the fast pass returns fewer characters than the configured minimum
- **THEN** the extractor runs a second time with OCR, that text is used, and the header records
  the fallback

#### Scenario: Forced OCR skips the fast pass
- **WHEN** `ratchet pdf` is run with the OCR flag
- **THEN** the extractor runs exactly once, with OCR, and the header records the forced mode

### Requirement: Extractor failure or timeout is a refusal, never a partial answer
An extractor that exits with an error, or that exceeds its configured timeout, SHALL be refused
and declared in one line with no stack trace. It SHALL NOT fall back to a partial or empty
answer.

#### Scenario: Extractor failure is refused and declared
- **WHEN** the extractor exits with an error
- **THEN** the call fails quoting the extractor's reason, and no sink file is written

#### Scenario: Extractor timeout is refused without a stack trace
- **WHEN** the extractor does not finish within its configured timeout
- **THEN** the call fails in one line naming the timeout, with no stack trace

### Requirement: Compact output — header and sink path, never the body
The terminal SHALL show only a bounded header (the input file, the pages requested, whether OCR
was used, the character count, and the sink path) — never the extracted text itself.

#### Scenario: Header-only output, never the body
- **WHEN** a PDF whose extracted text is long is extracted
- **THEN** the terminal shows the header and the sink path, and none of the extracted text
