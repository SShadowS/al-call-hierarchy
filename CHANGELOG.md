# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **External .app dependency support**: The server now resolves calls to procedures defined in compiled .app packages
  - Automatically parses `app.json` to discover declared dependencies
  - Finds matching .app files in the `.alpackages` folder with version matching
  - Extracts procedure definitions from `SymbolReference.json` inside .app files
  - Shows "(from AppName)" in call hierarchy for resolved external calls
  - Supports all standard BC object types: Codeunits, Tables, Pages, Reports, etc.

### New modules
- `app_package.rs` - Parser for .app files (ZIP with 40-byte NAVX header)
- `dependencies.rs` - Dependency discovery and resolution from app.json

### Dependencies
- Added `zip` crate for .app file extraction
- Added `roxmltree` crate for NavxManifest.xml parsing
