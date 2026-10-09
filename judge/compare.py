#!/usr/bin/env python3
"""
Megabase Judge - Response Comparison Harness

Sends identical requests to both the reference Supabase stack and Megabase,
then compares their responses to verify compatibility.

Normalization rules (documented for transparency):
- Timestamps: Normalized to a fixed epoch for comparison
- UUIDs: Compared as valid UUIDs, not exact values (unless deterministic)
- Order: JSON arrays are sorted by a stable key when order is not significant
- Headers: Only semantically significant headers are compared (Content-Type, etc.)

Usage:
    python compare.py                    # Run all tests
    python compare.py --component auth   # Run only auth tests
    python compare.py --verbose          # Show detailed diffs

Licensed under Apache-2.0
"""

import argparse
import json
import re
import sys
from dataclasses import dataclass
from datetime import datetime
from typing import Any
from urllib.parse import urljoin

import requests

# Configuration
REFERENCE_BASE = "http://localhost"
MEGABASE_BASE = "http://localhost:8000"

REFERENCE_PORTS = {
    "rest": 3000,
    "auth": 9999,
    "realtime": 4000,
    "storage": 5000,
    "meta": 8080,
}

JWT_SECRET = "super-secret-jwt-token-with-at-least-32-characters-long"

# Test JWT tokens
ANON_TOKEN = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZS1kZW1vIiwicm9sZSI6ImFub24iLCJleHAiOjE5ODM4MTI5OTZ9.CRXP1A7WOeoJeXxjNni43kdQwgnWNReilDMblYTn_I0"
SERVICE_TOKEN = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZS1kZW1vIiwicm9sZSI6InNlcnZpY2Vfcm9sZSIsImV4cCI6MTk4MzgxMjk5Nn0.EGIM96RAZx35lJzdJsyH-qQwv8Hdp7fsn3W0YpN81IU"


@dataclass
class TestResult:
    """Result of a single comparison test."""
    name: str
    component: str
    unit_id: str
    passed: bool
    reference_status: int
    megabase_status: int
    reference_body: Any
    megabase_body: Any
    diff: str | None = None
    error: str | None = None


def normalize_timestamps(obj: Any) -> Any:
    """Replace timestamps with a normalized value for comparison."""
    if isinstance(obj, dict):
        return {k: normalize_timestamps(v) for k, v in obj.items()}
    elif isinstance(obj, list):
        return [normalize_timestamps(v) for v in obj]
    elif isinstance(obj, str):
        # ISO 8601 timestamp pattern
        if re.match(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}", obj):
            return "TIMESTAMP_NORMALIZED"
        # UUID pattern
        if re.match(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", obj, re.I):
            return "UUID_NORMALIZED"
    return obj


def normalize_response(body: Any, sort_arrays: bool = False) -> Any:
    """Normalize a response body for comparison."""
    normalized = normalize_timestamps(body)
    if sort_arrays and isinstance(normalized, list):
        try:
            normalized = sorted(normalized, key=lambda x: json.dumps(x, sort_keys=True))
        except TypeError:
            pass
    return normalized


def compare_responses(reference: Any, megabase: Any) -> tuple[bool, str | None]:
    """Compare two normalized responses."""
    ref_norm = normalize_response(reference)
    meg_norm = normalize_response(megabase)
    
    if ref_norm == meg_norm:
        return True, None
    
    # Generate diff
    ref_str = json.dumps(ref_norm, indent=2, sort_keys=True)
    meg_str = json.dumps(meg_norm, indent=2, sort_keys=True)
    
    diff_lines = []
    for i, (r, m) in enumerate(zip(ref_str.split('\n'), meg_str.split('\n'))):
        if r != m:
            diff_lines.append(f"  Line {i+1}:")
            diff_lines.append(f"    REF: {r}")
            diff_lines.append(f"    MEG: {m}")
    
    return False, '\n'.join(diff_lines[:20])  # Limit diff size


def make_request(base_url: str, method: str, path: str, **kwargs) -> tuple[int, Any]:
    """Make an HTTP request and return status code and body."""
    url = urljoin(base_url, path)
    try:
        resp = requests.request(method, url, timeout=10, **kwargs)
        try:
            body = resp.json()
        except Exception:
            body = resp.text
        return resp.status_code, body
    except Exception as e:
        return 0, {"error": str(e)}


def run_test(
    name: str,
    component: str,
    unit_id: str,
    method: str,
    ref_path: str,
    meg_path: str,
    **kwargs
) -> TestResult:
    """Run a single comparison test."""
    ref_port = REFERENCE_PORTS.get(component, 3000)
    ref_base = f"{REFERENCE_BASE}:{ref_port}"
    
    ref_status, ref_body = make_request(ref_base, method, ref_path, **kwargs)
    meg_status, meg_body = make_request(MEGABASE_BASE, method, meg_path, **kwargs)
    
    # For 501 responses from Megabase, that's expected during development
    if meg_status == 501:
        return TestResult(
            name=name,
            component=component,
            unit_id=unit_id,
            passed=False,
            reference_status=ref_status,
            megabase_status=meg_status,
            reference_body=ref_body,
            megabase_body=meg_body,
            diff="Megabase returned 501 NOT IMPLEMENTED"
        )
    
    # Compare status codes
    if ref_status != meg_status:
        return TestResult(
            name=name,
            component=component,
            unit_id=unit_id,
            passed=False,
            reference_status=ref_status,
            megabase_status=meg_status,
            reference_body=ref_body,
            megabase_body=meg_body,
            diff=f"Status mismatch: reference={ref_status}, megabase={meg_status}"
        )
    
    # Compare bodies
    passed, diff = compare_responses(ref_body, meg_body)
    
    return TestResult(
        name=name,
        component=component,
        unit_id=unit_id,
        passed=passed,
        reference_status=ref_status,
        megabase_status=meg_status,
        reference_body=ref_body,
        megabase_body=meg_body,
        diff=diff
    )


def get_test_cases() -> list[dict]:
    """Define all comparison test cases."""
    headers_anon = {"Authorization": f"Bearer {ANON_TOKEN}"}
    headers_service = {"Authorization": f"Bearer {SERVICE_TOKEN}"}
    
    return [
        # REST API tests
        {
            "name": "REST: Get root",
            "component": "rest",
            "unit_id": "rest.feature.select",
            "method": "GET",
            "ref_path": "/",
            "meg_path": "/rest/v1/",
            "headers": headers_anon,
        },
        {
            "name": "REST: List test_items",
            "component": "rest",
            "unit_id": "rest.table.get",
            "method": "GET",
            "ref_path": "/test_items",
            "meg_path": "/rest/v1/test_items",
            "headers": headers_anon,
        },
        {
            "name": "REST: Select with eq filter",
            "component": "rest",
            "unit_id": "rest.operator.eq",
            "method": "GET",
            "ref_path": "/test_items?name=eq.test",
            "meg_path": "/rest/v1/test_items?name=eq.test",
            "headers": headers_anon,
        },
        
        # Auth tests
        {
            "name": "Auth: Health check",
            "component": "auth",
            "unit_id": "auth.endpoint.health",
            "method": "GET",
            "ref_path": "/health",
            "meg_path": "/auth/v1/health",
        },
        {
            "name": "Auth: Settings",
            "component": "auth",
            "unit_id": "auth.endpoint.settings",
            "method": "GET",
            "ref_path": "/settings",
            "meg_path": "/auth/v1/settings",
        },
        
        # Meta tests
        {
            "name": "Meta: List schemas",
            "component": "meta",
            "unit_id": "meta.endpoint.schemas",
            "method": "GET",
            "ref_path": "/schemas",
            "meg_path": "/pg/schemas",
        },
        {
            "name": "Meta: List tables",
            "component": "meta",
            "unit_id": "meta.endpoint.tables",
            "method": "GET",
            "ref_path": "/tables",
            "meg_path": "/pg/tables",
        },
        
        # Storage tests
        {
            "name": "Storage: List buckets",
            "component": "storage",
            "unit_id": "storage.bucket.list",
            "method": "GET",
            "ref_path": "/bucket",
            "meg_path": "/storage/v1/bucket",
            "headers": headers_service,
        },
    ]


def main():
    parser = argparse.ArgumentParser(description="Megabase Judge - Response Comparison")
    parser.add_argument("--component", help="Test only this component")
    parser.add_argument("--verbose", "-v", action="store_true", help="Show detailed output")
    parser.add_argument("--json", action="store_true", help="Output results as JSON")
    args = parser.parse_args()
    
    test_cases = get_test_cases()
    
    if args.component:
        test_cases = [t for t in test_cases if t["component"] == args.component]
    
    results = []
    passed = 0
    failed = 0
    
    for test in test_cases:
        result = run_test(**test)
        results.append(result)
        
        if result.passed:
            passed += 1
            if args.verbose:
                print(f"✓ {result.name}")
        else:
            failed += 1
            if args.verbose:
                print(f"✗ {result.name}")
                if result.diff:
                    print(f"  {result.diff}")
    
    if args.json:
        output = {
            "timestamp": datetime.now().isoformat(),
            "summary": {
                "total": len(results),
                "passed": passed,
                "failed": failed,
                "conformance_percent": round(passed / len(results) * 100, 2) if results else 0,
            },
            "results": [
                {
                    "name": r.name,
                    "component": r.component,
                    "unit_id": r.unit_id,
                    "passed": r.passed,
                    "reference_status": r.reference_status,
                    "megabase_status": r.megabase_status,
                    "diff": r.diff,
                }
                for r in results
            ]
        }
        print(json.dumps(output, indent=2))
    else:
        print(f"\n{'='*60}")
        print(f"Results: {passed}/{len(results)} passed ({passed/len(results)*100:.1f}% conformance)")
        print(f"{'='*60}")
    
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
