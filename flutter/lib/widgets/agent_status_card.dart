import 'dart:async';
import 'package:flutter/material.dart';

/// Generic agent status card that shows live last-event time and inference count.
/// [stream] is any stream from AgentStreamService.
class AgentStatusCard extends StatefulWidget {
  final String agentName;
  final String icon;
  final String description;
  final Stream<dynamic> stream;
  final Color color;

  const AgentStatusCard({
    super.key,
    required this.agentName,
    required this.icon,
    required this.description,
    required this.stream,
    required this.color,
  });

  @override
  State<AgentStatusCard> createState() => _AgentStatusCardState();
}

class _AgentStatusCardState extends State<AgentStatusCard> {
  int _inferenceCount = 0;
  DateTime? _lastDetection;
  bool _active = true;
  StreamSubscription<dynamic>? _sub;

  @override
  void initState() {
    super.initState();
    _sub = widget.stream.listen((_) {
      if (!mounted) return;
      setState(() {
        _inferenceCount++;
        _lastDetection = DateTime.now();
      });
    });
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: Colors.grey[900],
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: widget.color.withAlpha(80)),
      ),
      child: Row(
        children: [
          Text(widget.icon, style: const TextStyle(fontSize: 28)),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Text(widget.agentName,
                        style: TextStyle(
                            color: widget.color,
                            fontWeight: FontWeight.bold,
                            fontSize: 14)),
                    const SizedBox(width: 8),
                    _statusBadge(_active),
                  ],
                ),
                const SizedBox(height: 2),
                Text(widget.description,
                    style: const TextStyle(color: Colors.white54, fontSize: 11)),
                const SizedBox(height: 4),
                Row(
                  children: [
                    _pill('${_inferenceCount} events', Colors.white30),
                    const SizedBox(width: 6),
                    if (_lastDetection != null)
                      _pill(_fmtTime(_lastDetection!), Colors.white24),
                  ],
                ),
              ],
            ),
          ),
          Switch(
            value: _active,
            activeColor: widget.color,
            onChanged: (v) => setState(() => _active = v),
          ),
        ],
      ),
    );
  }

  Widget _statusBadge(bool active) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
        decoration: BoxDecoration(
          color: active ? Colors.green.withAlpha(40) : Colors.red.withAlpha(40),
          borderRadius: BorderRadius.circular(4),
        ),
        child: Text(
          active ? 'ACTIVE' : 'PAUSED',
          style: TextStyle(
              fontSize: 9,
              color: active ? Colors.greenAccent : Colors.redAccent,
              fontWeight: FontWeight.bold),
        ),
      );

  Widget _pill(String text, Color bg) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
        decoration: BoxDecoration(color: bg, borderRadius: BorderRadius.circular(4)),
        child: Text(text, style: const TextStyle(fontSize: 10, color: Colors.white70)),
      );

  String _fmtTime(DateTime t) =>
      '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}:${t.second.toString().padLeft(2, '0')}';
}
