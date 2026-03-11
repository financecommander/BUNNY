# BSA-45 — Household Remote Control & Systems Orchestration Directive

**System:** BUNNY–SWARM Platform
**Authority:** BUNNY Core (Home Operations) + BUNNY Shield (Safety, Identity & Access Enforcement)
**Component:** Jenny Household Control Layer
**Directive Type:** Remote Access, Device Command, Household Systems Orchestration, and Safety Governance

---

## 1. Purpose

This directive establishes the framework allowing the swarm, primarily through Jenny, to remotely monitor, control, and coordinate authorized household devices and systems.

The objective is to provide:
- Centralized home control
- Secure remote access
- Household automation
- Device orchestration
- Safety-aware command execution

Jenny becomes the household control agent for user-authorized homes and properties.

## 2. Scope

This directive applies only to:
- User-owned homes
- User-authorized devices
- Explicitly enrolled household systems
- Approved family or household members

This directive does not authorize control of:
- Third-party property
- Non-consenting devices
- Commercial/enterprise infrastructure unless separately approved
- Life-critical systems without explicit safeguards

## 3. Household Control Model

```
User
    ↓
Jenny Interface
    ↓
Household Control Gateway
    ↓
Device Registry + Policy Engine
    ↓
Authorized Home Systems
```

All remote actions must pass through the Household Control Gateway and policy checks.

## 4. Supported System Categories

**Access & Entry:** Smart locks, garage doors, gates, intercoms, door controllers

**Security:** Alarm systems, cameras, motion sensors, window/door sensors, security panels

**Lighting & Power:** Lights, switches, dimmers, outlets, circuit monitoring, backup power controls

**Climate & Environment:** Thermostats, HVAC zones, humidifiers, air purifiers, temperature sensors

**Appliances & Utilities:** Ovens, washers/dryers, water heaters, irrigation systems, pool systems

**Media & Presence:** Speakers, TV/media systems, room scenes, presence automations

**Monitoring Systems:** Leak detectors, smoke/CO detectors, energy monitors, freezer/fridge sensors, network health sensors

## 5. Core Capabilities

Jenny must support:
- Remote device control
- Room and zone control
- Whole-home modes
- Schedule-based automation
- Event-triggered actions
- State monitoring
- Alert delivery
- Command confirmation for sensitive actions

## 6. Household Modes

Supported modes: Home, Away, Night, Vacation, Guest, Emergency

A mode may trigger multiple device actions.

**Example (Away mode):** Lock doors, arm security, reduce HVAC load, turn off nonessential lights, enable alert monitoring.

## 7. Remote Control Classes

| Class | Risk Level | Examples |
|---|---|---|
| Class 1 | Low | Lights, media, room scenes |
| Class 2 | Moderate | Thermostats, garage doors, irrigation |
| Class 3 | Sensitive | Door locks, alarm state, camera privacy |
| Class 4 | Protected | Life-safety overrides, fire suppression, critical electrical |

Class 3 and 4 actions require stronger verification and policy review.

## 8. Authentication & Identity

Required controls: Named user identity, trusted device verification, strong authentication, optional step-up verification, household role permissions.

**Example roles:** Owner, adult household member, guest, child, service provider.

BUNNY Shield enforces all identity and access rules.

## 9. Device Registry

All controlled devices must be enrolled in a Household Device Registry.

| Field | Description |
|---|---|
| device_id | Unique identifier |
| device_type | Category |
| location | Room/zone |
| owner | Household owner |
| connector | Integration platform |
| risk_class | Control risk level |
| allowed_actions | Permitted operations |
| status | Current state |

Unregistered devices must not accept swarm control commands.

## 10. Safety Policies

Examples:
- Do not unlock doors without authorized identity
- Do not disable alarms without proper permissions
- Do not activate appliances in unsafe states
- Require confirmation for risky commands
- Prevent dangerous command chains

## 11. Privacy Rules

Privacy requirements: Minimal data collection, local processing where possible, encrypted storage and transport, camera/microphone privacy boundaries, user-visible activity logs.

Jenny must not expose home telemetry outside the authorized household context without permission.

## 12. Local Node Preference

Where possible, household control should run through a personal Calculus AI node in the home.

```
Jenny
    ↓
Home Node
    ↓
Local Device Connectors
    ↓
Household Systems
```

## 13. Remote Monitoring

Jenny may monitor device and home state for authorized users: door status, alarm status, temperature, camera event summaries, water leak alerts, power outage detection.

## 14. Alerts & Notifications

Delivery methods: Mobile alert, text summary, voice announcement, call escalation, dashboard notification.

## 15. Automation & Orchestration

Jenny may automate household workflows based on events and schedules. All automation must be reviewable and configurable by the user.

## 16. Emergency Handling

Emergency responses may include: user alert, family alert, device shutdown, mode transition, call escalation.

Jenny must not make unsafe assumptions in emergencies and should escalate clearly.

## 17. Connectors & Integrations

Modular connector domains: HomeKit, Google Home, Alexa, Ring, Nest, Hue, Z-Wave, Zigbee.

Each connector must expose standardized control operations.

## 18. Audit Logging

Required log fields: timestamp, user, agent, device, action, location, result, verification_level.

## 19. Integration with Other Directives

| Directive | Integration |
|---|---|
| BSA-32 | Jenny Personal Agent |
| BSA-33 | Household AI & Home Automation |
| BSA-30 | Calculus AI Node Deployment |
| BSA-38 | Unified Life & Work Intelligence |
| BSA-41 | Human-AI Collaboration Governance |

## 20. Expected Outcome

When implemented, this directive enables Jenny to function as a secure household control agent, a remote home operations interface, and a safety-aware home automation orchestrator.

## 21. Directive Activation

Directive BSA-45 is now active.

All swarm systems responsible for remote household control must comply with this framework.
