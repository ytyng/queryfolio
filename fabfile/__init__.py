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
        local('gh workflow run {} -f draft={}'.format(WORKFLOW, draft))
        if watch == 'true':
            # Give GitHub a moment to register the run, then print which run is
            # being followed (gh picks the most recent run for this workflow;
            # avoid dispatching twice in quick succession).
            local(
                'sleep 5 && '
                'RUN_ID="$(gh run list --workflow={wf} --limit 1 '
                "--json databaseId --jq '.[0].databaseId')\" && "
                'echo "Watching run $RUN_ID: '
                'https://github.com/ytyng/queryfolio/actions/runs/$RUN_ID" && '
                # --exit-status: fail the task if the build fails, so a broken
                # macOS build stops the release flow instead of looking done.
                'gh run watch "$RUN_ID" --exit-status'.format(wf=WORKFLOW)
            )


@task
def releases():
    """List the GitHub Releases (published macOS builds)."""
    with lcd(PROJECT_ROOT):
        local('gh release list')
