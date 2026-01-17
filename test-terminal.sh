#!/bin/bash
# Test script for PTY terminal WebSocket
# This tests the WebSocket layer without needing a full sandbox

set -e

echo "=== Microsandbox PTY Terminal Test ==="
echo ""

# Check if websocat is installed
if ! command -v websocat &> /dev/null; then
    echo "Installing websocat for WebSocket testing..."
    if command -v brew &> /dev/null; then
        brew install websocat
    elif command -v cargo &> /dev/null; then
        cargo install websocat
    else
        echo "Please install websocat: https://github.com/vi/websocat"
        exit 1
    fi
fi

# Configuration
SERVER_HOST="${SERVER_HOST:-localhost}"
SERVER_PORT="${SERVER_PORT:-5555}"
NAMESPACE="${NAMESPACE:-default}"
SANDBOX="${SANDBOX:-test-sandbox}"

# Check if server is running
echo "Checking server at $SERVER_HOST:$SERVER_PORT..."
if ! curl -s "http://$SERVER_HOST:$SERVER_PORT/api/v1/health" > /dev/null 2>&1; then
    echo "Server not running. Start with: msb server start --dev"
    echo ""
    echo "Or set SERVER_HOST/SERVER_PORT environment variables."
    exit 1
fi

echo "Server is running!"
echo ""

# Test WebSocket terminal
WS_URL="ws://$SERVER_HOST:$SERVER_PORT/ws/terminal/$NAMESPACE/$SANDBOX"
echo "Connecting to: $WS_URL"
echo ""
echo "Send JSON messages like:"
echo '  {"type":"init","shell":"/bin/bash","cols":80,"rows":24}'
echo '  {"type":"input","data":"<base64-encoded-data>"}'
echo '  {"type":"resize","cols":120,"rows":40}'
echo ""
echo "Press Ctrl+C to exit"
echo "---"

# Connect to WebSocket
websocat "$WS_URL"
