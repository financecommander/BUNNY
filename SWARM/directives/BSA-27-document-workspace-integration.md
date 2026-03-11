# BSA-27 — Document Retrieval, Generation & Workspace Integration Directive

**System:** Calculus AI Platform
**Authority:** BUNNY Core + BUNNY Shield
**Component:** Jack Document Intelligence Layer
**Directive Type:** Cross-Platform Document Access, Generation, Editing, and File Workflow Orchestration

---

## 1. Purpose

This directive defines how Jack supports document retrieval, creation, generation, updating, and routing across the user's file systems and productivity platforms.

Jack must allow users to request:
- Document retrieval
- Document generation
- Document editing
- Spreadsheet generation
- Presentation generation
- File conversion
- Document summarization
- Cross-platform save/export actions

Jack becomes a document operating layer across the system.

## 2. Supported Storage & Workspace Environments

### Storage Systems

| System | Type |
|---|---|
| Dropbox | Cloud storage |
| Google Drive | Cloud storage |
| OneDrive | Cloud storage |
| SharePoint | Enterprise storage |
| Local storage | File system |
| Network drives | Shared storage |
| Object storage | S3-compatible |
| Repositories | Git-based |

### Productivity Suites

| Platform | Formats |
|---|---|
| Google Workspace | Docs, Sheets, Slides |
| Microsoft Office | Word, Excel, PowerPoint |
| Other | PDF, CSV, TSV, JSON, Markdown |

### Internal Repositories

- GitHub repositories
- Internal document registry
- Knowledge archives
- Local project folders

## 3. Core User Requests

Jack must support natural requests:
- *"Find the underwriting deck from Google Drive."*
- *"Pull the latest project spreadsheet from Dropbox."*
- *"Create an Excel model for this deal."*
- *"Generate a Google Doc from these notes."*
- *"Turn this meeting transcript into a Word document."*
- *"Create a presentation from this strategy memo."*
- *"Summarize these PDFs and save the output to Drive."*

## 4. Document Workflow Model

```
Request Intake
    ↓
Source Resolution
    ↓
Permission Validation
    ↓
Document Retrieval or Generation
    ↓
Transformation / Editing
    ↓
Preview / Validation
    ↓
Save / Export / Route
    ↓
Logging & Memory Update
```

Each operation generates a Document Intelligence Record.

## 5. Source Resolution

Jack determines where documents live:
- Explicit source requests (*"in Dropbox"*)
- Implicit source matching
- File name / project context / folder context lookup
- Document type / recency matching

## 6. Retrieval Capabilities

**Search By:** File name, keyword, project, folder, type, date, author, semantic content

**Outputs:** Direct file match, top candidates, latest version, content summary, metadata

## 7. Generation Capabilities

### Supported Formats

| Format | Engine |
|---|---|
| Google Doc / Word | Document generation |
| Google Sheet / Excel | Spreadsheet generation |
| Google Slides / PowerPoint | Presentation generation |
| PDF | Export pipeline |
| Markdown / CSV | Structured export |

### Generation Types
Reports, briefing memos, financial models, meeting notes, proposals, decks, checklists, SOPs, templates

## 8. Editing & Transformation

**Examples:**
- Convert notes into formal memo
- Turn spreadsheet into presentation
- Convert PDF to editable text
- Merge multiple docs into summary
- Extract tables from reports

**Actions:** Reformat, summarize, expand, standardize, template-fill, convert, combine, version

## 9. Spreadsheet Intelligence

### Retrieval
Find financial models, budget sheets, KPI workbooks, cap tables, underwriting files

### Generation
Excel models, Google Sheets trackers, operating budgets, KPI dashboards, scenario analysis workbooks

### Actions
Populate formulas, create tabs, normalize data, structure assumptions, build summary sheets, export xlsx/csv

## 10. Presentation Intelligence

**Capabilities:**
- Find existing decks
- Create presentations from notes
- Update old decks
- Convert documents into decks

**Formats:** Google Slides, PowerPoint, PDF deck export

## 11. Local Storage Support

**Capabilities:** Search local folders, retrieve by path, generate into directories, watch project folders, organize by type

## 12. Cross-System Routing

**Examples:**
```
Dropbox PDF → summarize → save to Google Docs
Google Sheet → export to Excel → email as attachment
Local Markdown → convert to Word → save to SharePoint
Meeting transcript → generate summary deck → save to Drive
```

## 13. Connector Architecture

**Storage Connectors:**
```
/opt/swarm/connectors/dropbox/
/opt/swarm/connectors/google_drive/
/opt/swarm/connectors/onedrive/
/opt/swarm/connectors/sharepoint/
/opt/swarm/connectors/local_fs/
/opt/swarm/connectors/github/
```

**Generation Modules:**
```
/opt/swarm/docs/
/opt/swarm/spreadsheets/
/opt/swarm/slides/
/opt/swarm/pdf/
/opt/swarm/transform/
```

## 14. Workspace Actions

| Action | Description |
|---|---|
| find_document | Search across platforms |
| open_document | Open for viewing |
| summarize_document | Generate summary |
| generate_document | Create new document |
| update_document | Edit existing |
| convert_document | Change format |
| export_document | Export to target format |
| save_document | Save to destination |
| move_document | Transfer between platforms |
| version_document | Create new version |
| compare_documents | Diff two versions |

## 15. User Experience

Jack accepts plain language and infers: document type, target platform, output format, destination, project context, priority.

If underspecified, Jack makes a useful first pass: retrieve best matches, generate draft in default workspace, return preview before final export.

## 16. Permissions & Security

- Role-based access
- Connector-level permissions
- File and folder authorization
- Audit logging
- Sensitive document classification
- Secure credential handling

BUNNY Shield enforces. Jack must not access restricted files without authorization.

## 17. Versioning & Provenance

Each file workflow tracks:
- Source system and file
- Generation inputs
- Transformation steps
- Timestamp and user
- Output destination and version

Critical for legal, financial, and strategic documents.

## 18. Search & Ranking Logic

**Ranking Factors:** Name similarity, semantic relevance, project relevance, recency, author relevance, document type match, folder relevance, version freshness

Best candidate surfaced first, with alternates when confidence is lower.

## 19. Integration with Other Directives

| Directive | Integration |
|---|---|
| BSA-15 | Knowledge Governance |
| BSA-16 | Meeting Capture |
| BSA-17 | Email & Calendar |
| BSA-18 | Workflow Automation |
| BSA-23 | Personal Assistant |
| BSA-26 | Personal Knowledge Memory |

## 20. Expected Outcome

When implemented, Jack becomes:
- A cross-platform document retrieval engine
- A document generation assistant
- A spreadsheet and slide creation partner
- A file workflow orchestrator
- A workspace bridge across storage systems

## 21. Directive Activation

Directive BSA-27 is now active.

All document retrieval, generation, editing, export, and workspace integration systems must follow this directive.
