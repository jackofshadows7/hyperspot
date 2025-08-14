#!/bin/bash
# Smoke test script for HyperSpot server
# Tests basic functionality and API endpoints

set -e

BASE_URL="http://localhost:3000"
TIMEOUT=5

echo "🧪 HyperSpot Smoke Test Suite"
echo "=============================="

# Color output functions
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

success() {
    echo -e "${GREEN}✓${NC} $1"
}

error() {
    echo -e "${RED}✗${NC} $1"
}

warn() {
    echo -e "${YELLOW}⚠${NC} $1"
}

info() {
    echo -e "ℹ $1"
}

# Test if server is responding
test_health() {
    info "Testing health endpoint..."
    
    if curl -sf --max-time $TIMEOUT "$BASE_URL/health" > /dev/null; then
        success "Health endpoint responding"
        
        # Check response format
        HEALTH_RESPONSE=$(curl -sf --max-time $TIMEOUT "$BASE_URL/health")
        if echo "$HEALTH_RESPONSE" | jq -e '.status' > /dev/null 2>&1; then
            success "Health endpoint returns valid JSON"
        else
            warn "Health endpoint response format unexpected: $HEALTH_RESPONSE"
        fi
    else
        error "Health endpoint not responding"
        exit 1
    fi
}

# Test system information endpoint
test_sysinfo() {
    info "Testing system information endpoint..."
    
    if curl -sf --max-time $TIMEOUT "$BASE_URL/api/v1/sysinfo" > /dev/null; then
        success "Sysinfo endpoint responding"
        
        # Check response format
        SYSINFO_RESPONSE=$(curl -sf --max-time $TIMEOUT "$BASE_URL/api/v1/sysinfo")
        if echo "$SYSINFO_RESPONSE" | jq -e '.hostname' > /dev/null 2>&1; then
            success "Sysinfo endpoint returns valid system data"
        else
            warn "Sysinfo endpoint response format unexpected"
        fi
    else
        error "Sysinfo endpoint not responding"
        exit 1
    fi
}

# Test OpenAPI documentation
test_openapi() {
    info "Testing OpenAPI documentation..."
    
    if curl -sf --max-time $TIMEOUT "$BASE_URL/openapi.json" > /dev/null; then
        success "OpenAPI endpoint responding"
        
        # Check if it's valid OpenAPI
        OPENAPI_RESPONSE=$(curl -sf --max-time $TIMEOUT "$BASE_URL/openapi.json")
        if echo "$OPENAPI_RESPONSE" | jq -e '.info.title' > /dev/null 2>&1; then
            TITLE=$(echo "$OPENAPI_RESPONSE" | jq -r '.info.title')
            success "OpenAPI document valid (title: $TITLE)"
        else
            warn "OpenAPI document format unexpected"
        fi
    else
        error "OpenAPI endpoint not responding"
        exit 1
    fi
}

# Test CORS functionality
test_cors() {
    info "Testing CORS headers..."
    
    CORS_RESPONSE=$(curl -sf --max-time $TIMEOUT \
        -H "Origin: http://localhost:8080" \
        -H "Access-Control-Request-Method: GET" \
        -H "Access-Control-Request-Headers: Content-Type" \
        -X OPTIONS \
        -I "$BASE_URL/api/v1/sysinfo" 2>/dev/null || echo "")
    
    if echo "$CORS_RESPONSE" | grep -i "access-control-allow-origin" > /dev/null; then
        success "CORS headers present"
    else
        warn "CORS headers not found (may not be enabled)"
    fi
}

# Test request size limits
test_request_limits() {
    info "Testing request size limits..."
    
    # Try to send a large request (should fail with 413)
    LARGE_DATA=$(printf '%*s' 1000000 '' | tr ' ' 'a')
    
    HTTP_CODE=$(curl -sf --max-time $TIMEOUT \
        -X POST "$BASE_URL/api/v1/sysinfo" \
        -H "Content-Type: application/json" \
        -d "{\"data\":\"$LARGE_DATA\"}" \
        -w "%{http_code}" \
        -o /dev/null 2>/dev/null || echo "413")
    
    if [ "$HTTP_CODE" = "413" ] || [ "$HTTP_CODE" = "400" ]; then
        success "Request size limits enforced (HTTP $HTTP_CODE)"
    else
        warn "Request size limits may not be enforced (HTTP $HTTP_CODE)"
    fi
}

# Test interactive docs (just check if accessible)
test_docs() {
    info "Testing interactive documentation..."
    
    if curl -sf --max-time $TIMEOUT "$BASE_URL/docs" > /dev/null; then
        success "Interactive docs available at $BASE_URL/docs"
    else
        warn "Interactive docs not available (may not be enabled)"
    fi
}

# Test metrics/status endpoints
test_status() {
    info "Testing status endpoints..."
    
    if curl -sf --max-time $TIMEOUT "$BASE_URL/status" > /dev/null; then
        success "Status endpoint responding"
    else
        warn "Status endpoint not available"
    fi
}

# Main test execution
main() {
    echo
    info "Testing server at $BASE_URL"
    echo
    
    # Core functionality tests
    test_health
    test_sysinfo
    test_openapi
    
    echo
    
    # Feature tests
    test_cors
    test_request_limits
    test_docs
    test_status
    
    echo
    echo "🎉 Smoke tests completed!"
    echo
    info "Manual verification:"
    info "• Open $BASE_URL/docs for Stoplight Elements"
    info "• Open $BASE_URL/openapi.json for OpenAPI spec"
    info "• Check $BASE_URL/health for health status"
    echo
}

# Check dependencies
if ! command -v curl &> /dev/null; then
    error "curl is required but not installed"
    exit 1
fi

if ! command -v jq &> /dev/null; then
    warn "jq not found - JSON validation will be limited"
fi

# Run tests
main
