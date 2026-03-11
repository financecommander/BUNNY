#!/usr/bin/env python3
"""Fix Bunny Alpha to respond to channel messages, not just DMs and mentions."""
path = '/opt/bunny-alpha/bunny_alpha.py'
with open(path, 'r') as f:
    content = f.read()

# Replace the restrictive handler with one that responds to all messages
old = '''    if event.get("type") == "app_mention" or (
        event.get("type") == "message"
        and event.get("channel_type") in ("im", "mpim")
        and not event.get("bot_id")
        and not event.get("subtype")
    ):'''

new = '''    if (
        event.get("type") in ("app_mention", "message")
        and not event.get("bot_id")
        and not event.get("subtype")
    ):'''

if old in content:
    content = content.replace(old, new)
    with open(path, 'w') as f:
        f.write(content)
    print('Handler fixed - now responds to ALL messages (channels + DMs + mentions)')
else:
    print('ERROR: Pattern not found')
    for i, line in enumerate(content.split('\n')):
        if 'app_mention' in line or 'channel_type' in line:
            print(f'  Line {i}: {line}')
