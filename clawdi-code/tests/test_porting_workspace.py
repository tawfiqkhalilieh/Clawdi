from __future__ import annotations

import subprocess
import sys
import unittest
from pathlib import Path

from src.commands import PORTED_COMMANDS
from src.parity_audit import run_parity_audit
from src.port_manifest import build_port_manifest
from src.query_engine import QueryEnginePort
from src.tools import PORTED_TOOLS


class PortingWorkspaceTests(unittest.TestCase):
    def test_manifest_counts_python_files(self) -> None:
        manifest = build_port_manifest()
        self.assertGreaterEqual(manifest.total_python_files, 20)
        self.assertTrue(manifest.top_level_modules)

    def test_query_engine_summary_mentions_workspace(self) -> None:
        summary = QueryEnginePort.from_workspace().render_summary()
        self.assertIn('Python Porting Workspace Summary', summary)
        self.assertIn('Command surface:', summary)
        self.assertIn('Tool surface:', summary)

    def test_cli_summary_runs(self) -> None:
        result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'summary'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('Python Porting Workspace Summary', result.stdout)

    def test_parity_audit_runs(self) -> None:
        result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'parity-audit'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('Parity Audit', result.stdout)

    def test_root_file_coverage_is_complete_when_local_archive_exists(self) -> None:
        audit = run_parity_audit()
        if audit.archive_present:
            self.assertEqual(audit.root_file_coverage[0], audit.root_file_coverage[1])
            self.assertGreaterEqual(audit.directory_coverage[0], 28)
            self.assertGreaterEqual(audit.command_entry_ratio[0], 150)
            self.assertGreaterEqual(audit.tool_entry_ratio[0], 100)

    def test_command_and_tool_snapshots_are_nontrivial(self) -> None:
        self.assertGreaterEqual(len(PORTED_COMMANDS), 150)
        self.assertGreaterEqual(len(PORTED_TOOLS), 100)

    def test_commands_and_tools_cli_run(self) -> None:
        commands_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'commands', '--limit', '5', '--query', 'review'],
            check=True,
            capture_output=True,
            text=True,
        )
        tools_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'tools', '--limit', '5', '--query', 'MCP'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('Command entries:', commands_result.stdout)
        self.assertIn('Tool entries:', tools_result.stdout)

    def test_subsystem_packages_expose_archive_metadata(self) -> None:
        from src import assistant, bridge, utils

        self.assertGreater(assistant.MODULE_COUNT, 0)
        self.assertGreater(bridge.MODULE_COUNT, 0)
        self.assertGreater(utils.MODULE_COUNT, 100)
        self.assertTrue(utils.SAMPLE_FILES)

    def test_route_and_show_entry_cli_run(self) -> None:
        route_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'route', 'review MCP tool', '--limit', '5'],
            check=True,
            capture_output=True,
            text=True,
        )
        show_command = subprocess.run(
            [sys.executable, '-m', 'src.main', 'show-command', 'review'],
            check=True,
            capture_output=True,
            text=True,
        )
        show_tool = subprocess.run(
            [sys.executable, '-m', 'src.main', 'show-tool', 'MCPTool'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('review', route_result.stdout.lower())
        self.assertIn('review', show_command.stdout.lower())
        self.assertIn('mcptool', show_tool.stdout.lower())

    def test_bootstrap_cli_runs(self) -> None:
        result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'bootstrap', 'review MCP tool', '--limit', '5'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('Runtime Session', result.stdout)
        self.assertIn('Startup Steps', result.stdout)
        self.assertIn('Routed Matches', result.stdout)

    def test_bootstrap_session_tracks_turn_state(self) -> None:
        from src.runtime import PortRuntime

        session = PortRuntime().bootstrap_session('review MCP tool', limit=5)
        self.assertGreaterEqual(len(session.turn_result.matched_tools), 1)
        self.assertIn('Prompt:', session.turn_result.output)
        self.assertGreaterEqual(session.turn_result.usage.input_tokens, 1)

    def test_exec_command_and_tool_cli_run(self) -> None:
        command_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'exec-command', 'review', 'inspect security review'],
            check=True,
            capture_output=True,
            text=True,
        )
        tool_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'exec-tool', 'MCPTool', 'fetch resource list'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn("Mirrored command 'review'", command_result.stdout)
        self.assertIn("Mirrored tool 'MCPTool'", tool_result.stdout)

    def test_setup_report_and_registry_filters_run(self) -> None:
        setup_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'setup-report'],
            check=True,
            capture_output=True,
            text=True,
        )
        command_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'commands', '--limit', '5', '--no-plugin-commands'],
            check=True,
            capture_output=True,
            text=True,
        )
        tool_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'tools', '--limit', '5', '--simple-mode', '--no-mcp'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('Setup Report', setup_result.stdout)
        self.assertIn('Command entries:', command_result.stdout)
        self.assertIn('Tool entries:', tool_result.stdout)

    def test_load_session_cli_runs(self) -> None:
        from src.runtime import PortRuntime

        session = PortRuntime().bootstrap_session('review MCP tool', limit=5)
        session_id = Path(session.persisted_session_path).stem
        result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'load-session', session_id],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn(session_id, result.stdout)
        self.assertIn('messages', result.stdout)

    def test_tool_permission_filtering_cli_runs(self) -> None:
        result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'tools', '--limit', '10', '--deny-prefix', 'mcp'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('Tool entries:', result.stdout)
        self.assertNotIn('MCPTool', result.stdout)

    def test_turn_loop_cli_runs(self) -> None:
        result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'turn-loop', 'review MCP tool', '--max-turns', '2', '--structured-output'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('## Turn 1', result.stdout)
        self.assertIn('stop_reason=', result.stdout)

    def test_remote_mode_clis_run(self) -> None:
        remote_result = subprocess.run([sys.executable, '-m', 'src.main', 'remote-mode', 'workspace'], check=True, capture_output=True, text=True)
        ssh_result = subprocess.run([sys.executable, '-m', 'src.main', 'ssh-mode', 'workspace'], check=True, capture_output=True, text=True)
        teleport_result = subprocess.run([sys.executable, '-m', 'src.main', 'teleport-mode', 'workspace'], check=True, capture_output=True, text=True)
        self.assertIn('mode=remote', remote_result.stdout)
        self.assertIn('mode=ssh', ssh_result.stdout)
        self.assertIn('mode=teleport', teleport_result.stdout)

    def test_flush_transcript_cli_runs(self) -> None:
        result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'flush-transcript', 'review MCP tool'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('flushed=True', result.stdout)

    def test_command_graph_and_tool_pool_cli_run(self) -> None:
        command_graph = subprocess.run([sys.executable, '-m', 'src.main', 'command-graph'], check=True, capture_output=True, text=True)
        tool_pool = subprocess.run([sys.executable, '-m', 'src.main', 'tool-pool'], check=True, capture_output=True, text=True)
        self.assertIn('Command Graph', command_graph.stdout)
        self.assertIn('Tool Pool', tool_pool.stdout)

    def test_setup_report_mentions_deferred_init(self) -> None:
        result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'setup-report'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('Deferred init:', result.stdout)
        self.assertIn('plugin_init=True', result.stdout)

    def test_execution_registry_runs(self) -> None:
        from src.execution_registry import build_execution_registry

        registry = build_execution_registry()
        self.assertGreaterEqual(len(registry.commands), 150)
        self.assertGreaterEqual(len(registry.tools), 100)
        self.assertIn('Mirrored command', registry.command('review').execute('review security'))
        self.assertIn('Mirrored tool', registry.tool('MCPTool').execute('fetch mcp resources'))

    def test_bootstrap_graph_and_direct_modes_run(self) -> None:
        graph_result = subprocess.run([sys.executable, '-m', 'src.main', 'bootstrap-graph'], check=True, capture_output=True, text=True)
        direct_result = subprocess.run([sys.executable, '-m', 'src.main', 'direct-connect-mode', 'workspace'], check=True, capture_output=True, text=True)
        deep_link_result = subprocess.run([sys.executable, '-m', 'src.main', 'deep-link-mode', 'workspace'], check=True, capture_output=True, text=True)
        self.assertIn('Bootstrap Graph', graph_result.stdout)
        self.assertIn('mode=direct-connect', direct_result.stdout)
        self.assertIn('mode=deep-link', deep_link_result.stdout)


    def test_query_engine_stream_submit_runs(self) -> None:
        from src.query_engine import QueryEnginePort
        engine = QueryEnginePort.from_workspace()
        events = list(engine.stream_submit_message('hello', matched_commands=('review',)))
        
        event_types = [e['type'] for e in events]
        self.assertIn('message_start', event_types)
        self.assertIn('command_match', event_types)
        self.assertIn('message_delta', event_types)
        self.assertIn('message_stop', event_types)
        
        # Verify specific event content
        start_event = next(e for e in events if e['type'] == 'message_start')
        self.assertEqual(start_event['prompt'], 'hello')
        
        match_event = next(e for e in events if e['type'] == 'command_match')
        self.assertIn('review', match_event['commands'])

    def test_subsystems_cli_limit_runs(self) -> None:
        result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'subsystems', '--limit', '5'],
            check=True,
            capture_output=True,
            text=True,
        )
        lines = result.stdout.strip().split('\n')
        self.assertLessEqual(len(lines), 5)

    def test_turn_loop_live_gemini_flag_parsing(self) -> None:
        # We don't need a real API key to test that the flag is accepted and reaches the engine
        # But it will likely fail with an error message in the output
        result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'turn-loop', 'hello', '--max-turns', '1', '--live-gemini', '--gemini-model', 'test-model'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('## Turn 1', result.stdout)
        # It should either succeed (unlikely without key) or report an Error calling Gemini
        self.assertTrue('Error calling Gemini' in result.stdout or 'Matched commands' in result.stdout)

    def test_query_engine_config_overrides(self) -> None:
        from src.query_engine import QueryEngineConfig, QueryEnginePort
        config = QueryEngineConfig(max_turns=5, gemini_model='custom-model')
        engine = QueryEnginePort.from_workspace()
        engine.config = config
        self.assertEqual(engine.config.max_turns, 5)
        self.assertEqual(engine.config.gemini_model, 'custom-model')

if __name__ == '__main__':
    unittest.main()
