"""Fabric tasks for QueryFolio (Fabric3 / Fabric 1.x API).

Run `fab -l` to list tasks.

The macOS release build runs on GitHub Actions and is triggered manually from
here with `fab build_mac` (the Actions workflow is workflow_dispatch only).
"""
import os

from fabric.api import task, local, lcd

PROJECT_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

WORKFLOW = 'build-macos.yml'


@task
def dev():
    """Start the app in development mode (Rust build + Vite + native window)."""
    with lcd(PROJECT_ROOT):
        local('pnpm tauri dev')


@task
def check():
    """Type-check the frontend (svelte-check) and run the Rust unit tests."""
    with lcd(PROJECT_ROOT):
        local('pnpm check')
    with lcd(os.path.join(PROJECT_ROOT, 'src-tauri')):
        local('cargo test')


# Alias so `fab unittest` works (the standard task name in the fabfile skill).
@task
def unittest():
    """Run the Rust unit tests."""
    with lcd(os.path.join(PROJECT_ROOT, 'src-tauri')):
        local('cargo test')


@task
def build_local():
    """Build a signed release on this Mac (Developer ID, not notarized)."""
    with lcd(PROJECT_ROOT):
        local('pnpm tauri build')


@task
def build_mac(draft='true', watch='true'):
    """Trigger the macOS build+release GitHub Actions workflow (manual dispatch).

        fab build_mac                # draft release, then follow the run
        fab build_mac:draft=false    # publish the release directly (not a draft)
        fab build_mac:watch=false    # dispatch and return without following

    The workflow builds a universal (Apple Silicon + Intel) app and attaches the
    signed .dmg to a GitHub Release. See the `publish-macos-release` skill for
    the full publish-to-site runbook.
    """
    if draft not in ('true', 'false'):
        raise SystemExit("draft must be 'true' or 'false', got: {!r}".format(draft))
    with lcd(PROJECT_ROOT):
        if watch != 'true':
            local('gh workflow run {} -f draft={}'.format(WORKFLOW, draft))
            return
        # `gh workflow run` does not return the created run id, so record the
        # latest run id first, dispatch, then wait for a *new* run id to appear
        # and watch exactly that run (not an unrelated concurrent one).
        # `gh run watch --exit-status` fails the task if the build fails, so a
        # broken macOS build stops the release flow instead of looking done.
        local(
            'PREV=$(gh run list --workflow={wf} --limit 1 '
            "--json databaseId --jq '.[0].databaseId // empty') && "
            'gh workflow run {wf} -f draft={draft} && '
            'RUN_ID="" && '
            'for i in $(seq 1 20); do sleep 3; '
            'RID=$(gh run list --workflow={wf} --limit 1 '
            "--json databaseId --jq '.[0].databaseId // empty'); "
            'if [ -n "$RID" ] && [ "$RID" != "$PREV" ]; then RUN_ID="$RID"; break; fi; '
            'done && '
            'if [ -z "$RUN_ID" ]; then '
            'echo "Could not find the dispatched run; check: gh run list --workflow={wf}"; '
            'exit 1; fi && '
            'echo "Watching run $RUN_ID: '
            'https://github.com/ytyng/queryfolio/actions/runs/$RUN_ID" && '
            'gh run watch "$RUN_ID" --exit-status'.format(wf=WORKFLOW, draft=draft)
        )


@task
def releases():
    """List the GitHub Releases (published macOS builds)."""
    with lcd(PROJECT_ROOT):
        local('gh release list')
