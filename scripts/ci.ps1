# HyperSpot CI Script for Windows PowerShell
# Provides equivalent functionality to Makefile for Windows development

param(
    [Parameter(Position=0)]
    [string]$Command = "help",
    [switch]$Fix
)

function Write-Step {
    param([string]$Message)
    Write-Host "$Message" -ForegroundColor Cyan
}

function Write-Success {
    param([string]$Message)
    Write-Host "$Message" -ForegroundColor Green
}

function Write-Error {
    param([string]$Message)
    Write-Host "$Message" -ForegroundColor Red
}

function Invoke-Format {
    Write-Step "Running cargo fmt..."
    if ($Fix) {
        cargo fmt --all
        if ($LASTEXITCODE -eq 0) {
            Write-Success "Code formatted successfully"
        } else {
            Write-Error "Format failed"
            exit 1
        }
    } else {
        cargo fmt --all -- --check
        if ($LASTEXITCODE -eq 0) {
            Write-Success "Code formatting is correct"
        } else {
            Write-Error "Code formatting issues found. Run: ./scripts/ci.ps1 fmt -Fix"
            exit 1
        }
    }
}

function Invoke-Clippy {
    Write-Step "Running cargo clippy..."
    if ($Fix) {
        cargo clippy --workspace --all-targets --fix --allow-dirty
        if ($LASTEXITCODE -eq 0) {
            Write-Success "Clippy issues fixed successfully"
        } else {
            Write-Error "Clippy fix failed"
            exit 1
        }
    } else {
        cargo clippy --workspace --all-targets -- -D warnings
        if ($LASTEXITCODE -eq 0) {
            Write-Success "No clippy warnings found"
        } else {
            Write-Error "Clippy warnings found. Run: ./scripts/ci.ps1 clippy -Fix"
            exit 1
        }
    }
}

function Invoke-Test {
    Write-Step "Running cargo test..."
    cargo test --workspace
    if ($LASTEXITCODE -eq 0) {
        Write-Success "All tests passed"
    } else {
        Write-Error "Some tests failed"
        exit 1
    }
}

function Invoke-Audit {
    Write-Step "Running cargo audit..."
    
    # Check if cargo-audit is installed
    cargo audit --version > $null 2>&1
    if ($LASTEXITCODE -ne 0) {
        Write-Step "Installing cargo-audit..."
        cargo install cargo-audit
        if ($LASTEXITCODE -ne 0) {
            Write-Error "Failed to install cargo-audit"
            exit 1
        }
    }
    
    cargo audit
    if ($LASTEXITCODE -eq 0) {
        Write-Success "No security vulnerabilities found"
    } else {
        Write-Error "Security vulnerabilities found"
        exit 1
    }
}

function Invoke-Deny {
    Write-Step "Running cargo deny..."
    
    # Check if cargo-deny is installed
    cargo deny --version > $null 2>&1
    if ($LASTEXITCODE -ne 0) {
        Write-Step "Installing cargo-deny..."
        cargo install cargo-deny
        if ($LASTEXITCODE -ne 0) {
            Write-Error "Failed to install cargo-deny"
            exit 1
        }
    }
    
    cargo deny check
    if ($LASTEXITCODE -eq 0) {
        Write-Success "No licensing or dependency issues found"
    } else {
        Write-Error "Licensing or dependency issues found"
        exit 1
    }
}

function Invoke-Security {
    Write-Step "Running security checks..."
    Invoke-Audit
    Invoke-Deny
    Write-Success "All security checks passed"
}

function Invoke-Check {
    Write-Step "Running full check suite..."
    Invoke-Format
    Invoke-Clippy
    Invoke-Test
    Invoke-Security
    Write-Success "All checks passed!"
}

function Invoke-Quickstart {
    Write-Step "Starting HyperSpot in quickstart mode..."
    
    if (!(Test-Path "data")) {
        New-Item -ItemType Directory -Path "data" | Out-Null
        Write-Step "Created data directory"
    }
    
    Write-Step "Starting server with quickstart configuration..."
    cargo run --bin hyperspot-server -- --config config/quickstart.yaml run
}

function Show-Help {
    Write-Host @"
HyperSpot CI Script for Windows PowerShell

Usage: ./scripts/ci.ps1 <command> [options]

Commands:
  fmt          Check code formatting (use -Fix to auto-format)
  clippy       Run Clippy linter (use -Fix to auto-fix)
  test         Run all tests
  audit        Run security audit
  deny         Check licenses and dependencies
  security     Run all security checks (audit + deny)
  check        Run all checks (fmt + clippy + test + security)
  quickstart   Start server in development mode
  help         Show this help message

Options:
  -Fix         Apply automatic fixes where possible

Examples:
  ./scripts/ci.ps1 check              # Run full CI pipeline
  ./scripts/ci.ps1 fmt -Fix           # Auto-format code
  ./scripts/ci.ps1 clippy -Fix        # Auto-fix clippy issues
  ./scripts/ci.ps1 quickstart         # Start development server

Note: For full make compatibility, install make via:
  winget install GnuWin32.Make
  # or use Chocolatey: choco install make
"@ -ForegroundColor White
}

# Main command dispatch
switch ($Command.ToLower()) {
    "fmt" { Invoke-Format }
    "clippy" { Invoke-Clippy }
    "test" { Invoke-Test }
    "audit" { Invoke-Audit }
    "deny" { Invoke-Deny }
    "security" { Invoke-Security }
    "check" { Invoke-Check }
    "quickstart" { Invoke-Quickstart }
    "help" { Show-Help }
    default { 
        Write-Error "Unknown command: $Command"
        Show-Help
        exit 1
    }
}
