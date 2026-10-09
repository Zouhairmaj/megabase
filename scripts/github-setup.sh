#!/usr/bin/env bash
# Megabase GitHub Project Setup
#
# This script sets up GitHub milestones, labels, and project board.
# Requires: gh CLI authenticated with appropriate permissions
#
# Usage:
#   ./scripts/github-setup.sh
#
# Note: Some operations require admin/owner permissions.
# Run what you can; list what fails for human follow-up.

set -euo pipefail

REPO="AgenP/megabase"  # Update this to your repo

echo "=== Megabase GitHub Setup ==="
echo ""

# Check gh CLI
if ! command -v gh &> /dev/null; then
    echo "Error: gh CLI not found. Install from https://cli.github.com/"
    exit 1
fi

# Check auth
if ! gh auth status &> /dev/null; then
    echo "Error: Not authenticated. Run 'gh auth login' first."
    exit 1
fi

echo "Creating labels..."
LABELS=(
    "component:rest,0366d6,REST API (PostgREST)"
    "component:auth,0366d6,Authentication (GoTrue)"
    "component:realtime,0366d6,Realtime"
    "component:storage,0366d6,Storage"
    "component:functions,0366d6,Edge Functions"
    "component:pooler,0366d6,Connection Pooler (Supavisor)"
    "component:meta,0366d6,Postgres Meta"
    "component:studio,0366d6,Studio"
    "component:core,0366d6,Core shared code"
    "component:judge,0366d6,Judge comparison system"
    "level:1,c5def5,Level 1 - REST + Auth basics"
    "level:2,c5def5,Level 2 - OAuth + Storage"
    "level:3,c5def5,Level 3 - Realtime"
    "level:4,c5def5,Level 4 - Functions + Meta + Studio test"
    "level:5,c5def5,Level 5 - Studio rewrite (stretch)"
    "type:feature,a2eeef,New feature implementation"
    "type:bug,d73a4a,Bug fix"
    "type:spec,fbca04,Specification writing"
    "type:infra,fef2c0,Infrastructure/tooling"
    "type:investigation,e99695,Research/investigation"
    "blocked,b60205,Blocked on something"
    "judge-dispute,ff7619,Dispute with judge results"
    "needs-human,d4c5f9,Requires human input"
)

for label_def in "${LABELS[@]}"; do
    IFS=',' read -r name color desc <<< "$label_def"
    echo "  Creating label: $name"
    gh label create "$name" --color "$color" --description "$desc" --repo "$REPO" 2>/dev/null || \
        gh label edit "$name" --color "$color" --description "$desc" --repo "$REPO" 2>/dev/null || \
        echo "    (already exists or no permission)"
done

echo ""
echo "Creating milestones..."
MILESTONES=(
    "Level 1|REST API (PostgREST) + email/password auth. Conformance threshold: 80%"
    "Level 2|OAuth providers, magic links, OTP, Storage. Conformance threshold: 80%"
    "Level 3|Realtime: database changes, broadcast, presence. Conformance threshold: 80%"
    "Level 4|Edge Functions, Pooler, Postgres Meta, Studio test. Conformance threshold: 80%"
    "Level 5|Studio rewrite (stretch goal). Conformance threshold: 80%"
)

for milestone_def in "${MILESTONES[@]}"; do
    IFS='|' read -r title desc <<< "$milestone_def"
    echo "  Creating milestone: $title"
    gh api repos/$REPO/milestones -f title="$title" -f description="$desc" 2>/dev/null || \
        echo "    (already exists or no permission)"
done

echo ""
echo "=== Manual Setup Required ==="
echo ""
echo "The following require admin/owner access in GitHub settings:"
echo ""
echo "1. Branch Protection Rules (Settings > Branches > Add rule for 'main'):"
echo "   - Require pull request reviews before merging"
echo "   - Require status checks to pass: 'Build', 'Check Protected Files'"
echo "   - Require branches to be up to date"
echo "   - Include administrators"
echo ""
echo "2. GitHub Project Board:"
echo "   - Create a project at https://github.com/orgs/AgenP/projects or repo projects"
echo "   - Columns: Backlog, Ready, In Progress, In Review, Done, Blocked"
echo "   - Custom fields: component, level, coverage_delta, conformance_delta, effort"
echo ""
echo "3. CODEOWNERS enforcement:"
echo "   - Settings > Branches > Edit 'main' rule"
echo "   - Enable 'Require review from Code Owners'"
echo ""
echo "4. Bot account (optional):"
echo "   - Create a GitHub App or bot account for agent actions"
echo "   - Add as collaborator with appropriate permissions"
echo ""
echo "=== Setup Complete ==="
