#!/usr/bin/env python3
"""Add debug logging to bunny_alpha.py"""
path = '/opt/bunny-alpha/bunny_alpha.py'
with open(path, 'r') as f:
    content = f.read()

old = '    # Process event\n    event = data.get("event", {})\n    event_id = data.get("event_id", "")'

new = '''    # Process event
    event = data.get("event", {})
    event_id = data.get("event_id", "")

    log.info(
        "Event received: type=%s subtype=%s channel_type=%s bot_id=%s user=%s text=%s",
        event.get("type"), event.get("subtype"), event.get("channel_type"),
        event.get("bot_id"), event.get("user"), str(event.get("text", ""))[:50]
    )'''

if old in content:
    content = content.replace(old, new)
    with open(path, 'w') as f:
        f.write(content)
    print('Debug logging added successfully')
else:
    print('ERROR: Pattern not found')
    # Find the approximate location
    lines = content.split('\n')
    for i, line in enumerate(lines):
        if 'Process event' in line or 'event_id' in line:
            print(f'  Line {i}: {line}')
