# BSA-16 — Jack Meeting Capture, Dictation & Work System Update Directive

**System:** Calculus AI Platform
**Authority:** BUNNY Core + BUNNY Shield
**Component:** Jack Meeting Intelligence Layer
**Directive Type:** Meeting Dictation, Transcription, Action Extraction, and Work System Integration

---

## 1. Purpose

This directive defines how Jack participates in meetings, captures discussions, extracts decisions, and updates operational systems such as Asana.

Jack must function as a meeting intelligence assistant that can:
- Transcribe meetings
- Summarize discussions
- Extract tasks and decisions
- Update project management systems
- Track follow-ups automatically

Jack becomes the organizational meeting recorder and action coordinator.

## 2. Meeting Intelligence Workflow

```
Live Audio
    ↓
Speech Recognition
    ↓
Speaker Identification
    ↓
Transcript Generation
    ↓
Topic Segmentation
    ↓
Decision & Task Extraction
    ↓
Summary Generation
    ↓
System Updates (Asana, Docs, etc.)
```

Each meeting generates a Meeting Intelligence Record.

## 3. Meeting Capture Methods

### Live Meeting Participation

| Platform | Integration |
|---|---|
| Zoom | Bot participant |
| Google Meet | Bot participant |
| Microsoft Teams | Bot participant |
| Phone calls | VoiceAI pipeline |
| In-person | Microphone capture |

Jack joins meetings as a recording assistant.

### Voice Dictation

Users may dictate directly to Jack:

*"Jack, note that we approved the new pricing model."*

Jack converts dictated notes into structured meeting records.

## 4. Speech Recognition Layer

**Recommended Technologies:** Whisper, Deepgram, AssemblyAI

**Required Features:**
- Speaker separation
- Real-time transcription
- High accuracy in noisy environments

**Module:** `/opt/swarm/meeting/stt/`

## 5. Speaker Identification

Jack must identify participants via:
- Voice recognition
- Meeting platform participant data
- Manual identification

**Example Transcript:**
```
Alex: We should launch next quarter.
Stephanie: I agree but we need budget approval.
```

## 6. Transcript Generation

**Transcript Record:**

| Field | Description |
|---|---|
| meeting_id | Unique identifier |
| participants | List of speakers |
| timestamp | Per-utterance timing |
| speaker | Speaker identity |
| speech_text | Transcribed text |

Transcripts must be stored in the organizational knowledge system (BSA-15).

## 7. Topic Segmentation

Jack identifies topic segments:
- Budget discussion
- Project updates
- Product planning
- Technical issues

Segmenting conversations improves task extraction accuracy.

## 8. Task Extraction

Jack identifies actionable items from conversations.

**Example:**

| Meeting Statement | Extracted Task |
|---|---|
| *"Alex will prepare the financial report by Friday."* | Task: Prepare financial report, Owner: Alex, Deadline: Friday |

Tasks become structured task objects.

## 9. Decision Extraction

Jack detects key decisions:

```
Decision: Launch marketing campaign in Q3
Approved by: Executive team
```

Decisions are stored in the knowledge graph.

## 10. Meeting Summary Generation

**Summary Structure:**
- Meeting overview
- Key topics discussed
- Decisions made
- Tasks assigned
- Next steps

## 11. Asana Integration

Jack automatically updates task systems.

**Task Creation Example:**

| Field | Value |
|---|---|
| Task Title | Prepare Financial Report |
| Assigned To | Alex |
| Due Date | Friday |
| Project | Q3 Launch |

**Integration Methods:** Asana API, webhook automation, task synchronization

**Module:** `/opt/swarm/integrations/asana/`

## 12. Other System Integrations

Jack should also update:
- Slack or Teams (discussion summaries)
- Document systems (meeting notes)
- Project dashboards
- CRM systems

**Module:** `/opt/swarm/integrations/`

## 13. Meeting Dashboard

**Dashboard Features:**
- Meeting transcripts
- Summary reports
- Action items
- Decision history
- Task status

## 14. Real-Time Meeting Assistance

During meetings Jack assists:

*"Jack, summarize the last five minutes."*
*"Jack, list action items so far."*

Jack provides responses instantly.

## 15. Post-Meeting Follow-Ups

After meetings Jack sends:
- Summary email
- Task assignments
- Reminders
- Document links

Follow-ups ensure accountability.

## 16. Security & Privacy

**Security Rules:**
- Record meetings only with consent
- Encrypt stored transcripts
- Restrict access based on roles

Enforcement is handled by BUNNY Shield.

## 17. Learning from Meetings

Meeting data feeds the organizational intelligence system:
- Project progress
- Organizational priorities
- Decision patterns
- Team expertise

This improves future strategic analysis.

## 18. Expected Outcome

When implemented, Jack becomes:
- An AI meeting recorder
- A project task generator
- A decision tracking system
- A collaboration intelligence assistant

Teams no longer need manual meeting notes.

## 19. Directive Activation

Directive BSA-16 is now active.

All meeting capture and task-automation systems must comply with this directive.
