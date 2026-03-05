import 'dart:async';
import 'package:flutter/material.dart';
import '../models/threat_event.dart';
import '../models/ai_guardian_alert.dart';
import '../models/sandbox_report.dart';
import '../services/agent_stream_service.dart';

class _FeedEntry {
  final DateTime time;
  final String agent;
  final String severity;
  final String message;
  final Color color;

  const _FeedEntry({
    required this.time,
    required this.agent,
    required this.severity,
    required this.message,
    required this.color,
  });
}

/// Auto-scrolling real-time anomaly feed with filter controls.
class AnomalyFeed extends StatefulWidget {
  const AnomalyFeed({super.key});

  @override
  State<AnomalyFeed> createState() => _AnomalyFeedState();
}

class _AnomalyFeedState extends State<AnomalyFeed> {
  final List<_FeedEntry> _entries = [];
  final _scrollController = ScrollController();
  bool _paused = false;
  String _agentFilter = 'All';
  String _severityFilter = 'All';

  static const _maxEntries = 100;
  final _agents = ['All', 'Threat Hunter', 'Sandbox', 'AI Guardian', 'IoT Firewall'];
  final _severities = ['All', 'CRITICAL', 'MEDIUM', 'ANOMALY', 'REPORT', 'ALERT'];

  final List<StreamSubscription<dynamic>> _subs = [];

  @override
  void initState() {
    super.initState();
    final svc = AgentStreamService();

    _subs.add(svc.threatHunterStream().listen((ThreatEvent e) => _add(
          agent: 'Threat Hunter',
          severity: e.severityLabel,
          message: e.description,
          color: _sevColor(e.severity),
        )));

    _subs.add(svc.sandboxStream().listen((SandboxReport r) => _add(
          agent: 'Sandbox',
          severity: 'REPORT',
          message: '${r.filename}: ${r.verdict.toUpperCase()} '
              '(score ${(r.maliciousScore * 100).toStringAsFixed(0)}%)',
          color: r.verdict == 'malicious' ? Colors.red : Colors.greenAccent,
        )));

    _subs.add(svc.aiGuardianStream().listen((AiGuardianAlert a) => _add(
          agent: 'AI Guardian',
          severity: 'ALERT',
          message: '${a.action}: ${a.reason}',
          color: a.blocked ? Colors.orange : Colors.blueAccent,
        )));
  }

  void _add({
    required String agent,
    required String severity,
    required String message,
    required Color color,
  }) {
    if (_paused || !mounted) return;
    setState(() {
      _entries.insert(
          0,
          _FeedEntry(
              time: DateTime.now(),
              agent: agent,
              severity: severity,
              message: message,
              color: color));
      if (_entries.length > _maxEntries) _entries.removeLast();
    });
    // Auto-scroll
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_scrollController.hasClients && !_paused) {
        _scrollController.animateTo(0,
            duration: const Duration(milliseconds: 200), curve: Curves.easeOut);
      }
    });
  }

  @override
  void dispose() {
    for (final s in _subs) {
      s.cancel();
    }
    _scrollController.dispose();
    super.dispose();
  }

  List<_FeedEntry> get _filtered => _entries.where((e) {
        if (_agentFilter != 'All' && e.agent != _agentFilter) return false;
        if (_severityFilter != 'All' && e.severity != _severityFilter) return false;
        return true;
      }).toList();

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        // Controls
        Row(
          children: [
            Expanded(child: _dropdown('Agent', _agentFilter, _agents, (v) => setState(() => _agentFilter = v!))),
            const SizedBox(width: 8),
            Expanded(child: _dropdown('Severity', _severityFilter, _severities, (v) => setState(() => _severityFilter = v!))),
            const SizedBox(width: 8),
            IconButton(
              icon: Icon(_paused ? Icons.play_arrow : Icons.pause,
                  color: Colors.deepOrangeAccent),
              onPressed: () => setState(() => _paused = !_paused),
              tooltip: _paused ? 'Resume' : 'Pause',
            ),
          ],
        ),
        const SizedBox(height: 4),
        Expanded(
          child: Container(
            decoration: BoxDecoration(
              color: Colors.black,
              borderRadius: BorderRadius.circular(8),
              border: Border.all(color: Colors.grey.withAlpha(60)),
            ),
            child: _filtered.isEmpty
                ? const Center(
                    child: Text('Waiting for events…',
                        style: TextStyle(color: Colors.white38)))
                : ListView.builder(
                    controller: _scrollController,
                    itemCount: _filtered.length,
                    itemBuilder: (ctx, i) => _entryRow(_filtered[i]),
                  ),
          ),
        ),
      ],
    );
  }

  Widget _entryRow(_FeedEntry e) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
      child: RichText(
        text: TextSpan(
          style: const TextStyle(fontFamily: 'monospace', fontSize: 11),
          children: [
            TextSpan(
                text: '[${_fmtTime(e.time)}] ',
                style: const TextStyle(color: Colors.white38)),
            TextSpan(
                text: '[${e.agent}] ',
                style: const TextStyle(color: Colors.cyanAccent)),
            TextSpan(
                text: '[${e.severity}] ',
                style: TextStyle(color: e.color, fontWeight: FontWeight.bold)),
            TextSpan(
                text: e.message,
                style: const TextStyle(color: Colors.white70)),
          ],
        ),
        overflow: TextOverflow.ellipsis,
      ),
    );
  }

  Widget _dropdown(String hint, String value, List<String> options,
      ValueChanged<String?> onChanged) {
    return DropdownButton<String>(
      value: value,
      isExpanded: true,
      dropdownColor: Colors.grey[900],
      style: const TextStyle(fontSize: 11, color: Colors.white70),
      underline: Container(height: 1, color: Colors.deepOrange.withAlpha(80)),
      items: options.map((o) => DropdownMenuItem(value: o, child: Text(o))).toList(),
      onChanged: onChanged,
    );
  }

  Color _sevColor(ThreatSeverity s) {
    switch (s) {
      case ThreatSeverity.critical:
        return Colors.red;
      case ThreatSeverity.medium:
        return Colors.orange;
      case ThreatSeverity.anomaly:
        return Colors.greenAccent;
    }
  }

  String _fmtTime(DateTime t) =>
      '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}:${t.second.toString().padLeft(2, '0')}';
}
