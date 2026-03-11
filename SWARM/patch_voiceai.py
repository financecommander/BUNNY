#!/usr/bin/env python3
"""Patch VoiceAI orchestrator and CRM adapter to add JACK model."""
import subprocess

# Patch orchestrator readOnlyMap
with open('/opt/voiceai/src/orchestrator/orchestrator.ts', 'r') as f:
    content = f.read()

# Find the readOnlyMap closing for LOAN_SERVICING and add JACK
# Look for the pattern after getReadOnlyTools
old_readonly_end = """      LOAN_SERVICING: [
        { name: 'getLoanDetails', description: 'Get loan details and payment history', parameters: { customerId: 'string' } },
        { name: 'getNextPayment', description: 'Get next payment date and amount', parameters: { customerId: 'string' } },
        { name: 'getEscrowDetails', description: 'Get escrow account details', parameters: { customerId: 'string' } },
      ],
    };"""

new_readonly_end = """      LOAN_SERVICING: [
        { name: 'getLoanDetails', description: 'Get loan details and payment history', parameters: { customerId: 'string' } },
        { name: 'getNextPayment', description: 'Get next payment date and amount', parameters: { customerId: 'string' } },
        { name: 'getEscrowDetails', description: 'Get escrow account details', parameters: { customerId: 'string' } },
      ],
      JACK: [
        { name: 'listEmails', description: 'List inbox emails', parameters: { userId: 'string' } },
        { name: 'systemStatus', description: 'Get SWARM system status', parameters: {} },
      ],
    };"""

if old_readonly_end in content:
    content = content.replace(old_readonly_end, new_readonly_end)
    print('readOnlyMap patched')
else:
    print('readOnlyMap pattern not found, trying line-by-line')
    lines = content.split('\n')
    for i, line in enumerate(lines):
        if 'getReadOnlyTools' in line:
            # Find LOAN_SERVICING block end after this
            for j in range(i, min(i+100, len(lines))):
                if j > i and 'LOAN_SERVICING:' in lines[j]:
                    # Find closing ]
                    for k in range(j, min(j+10, len(lines))):
                        if lines[k].strip() == '],':
                            lines.insert(k+1, "      JACK: [")
                            lines.insert(k+2, "        { name: 'listEmails', description: 'List inbox emails', parameters: { userId: 'string' } },")
                            lines.insert(k+3, "        { name: 'systemStatus', description: 'Get SWARM system status', parameters: {} },")
                            lines.insert(k+4, "      ],")
                            print(f'readOnlyMap patched at line {k+1}')
                            content = '\n'.join(lines)
                            break
                    break
            break

with open('/opt/voiceai/src/orchestrator/orchestrator.ts', 'w') as f:
    f.write(content)

# Build
result = subprocess.run(['npx', 'tsc'], cwd='/opt/voiceai', capture_output=True, text=True)
if result.returncode == 0:
    print('BUILD SUCCESS')
else:
    errors = []
    for line in result.stdout.split('\n'):
        if 'error TS' in line:
            errors.append(line.strip())
    for e in errors[:10]:
        print(e)
    print(f'Total errors: {len(errors)}')
