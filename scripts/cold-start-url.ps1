# Shared PostgreSQL guard for the embedded cold start.
# A disposable name is not enough: the host must be the literal loopback form
# 127.0.0.1, localhost, or [::1]. The host is not percent-decoded, because
# sqlx-postgres 0.8.6 connects to url.host_str() as written. A host containing
# "%" is refused, as is every other spelling, including socket paths. The URL
# user must already be a disposable test role. The scripts do not read pg_roles
# and do not verify that role's privileges. Any query string is refused,
# including an empty query and parameters such as dbname, host, hostaddr, port,
# user, password, options, and application_name. The query is not removed before
# this decision. The database path is decoded once. User, password, and host
# never satisfy the database-name rule. Unsupported or unclear URLs are refused.

function ConvertFrom-ColdStartPercentEncoding {
    param([string]$Value)
    if ($Value -match '%(?![0-9A-Fa-f]{2})' -or $Value -match '(?i)%00') {
        throw "invalid percent-encoding"
    }
    return [Uri]::UnescapeDataString($Value)
}

function Get-ColdStartDatabaseRejection {
    param([string]$Url)
    if ($Url -match '[\s#]') {
        return "database URL is unclear"
    }
    if ($Url -notmatch '://') {
        return "unsupported or unclear database URL"
    }
    $scheme = ($Url -split '://', 2)[0].ToLowerInvariant()
    if ($scheme -eq "sqlite") {
        return $null
    }
    if ($scheme -ne "postgres" -and $scheme -ne "postgresql") {
        return "unsupported database URL scheme"
    }
    if ($Url.Contains("?")) {
        return "database URL query is not accepted"
    }

    $rest = ($Url -split '://', 2)[1]
    $slash = $rest.IndexOf("/")
    if ($slash -lt 0) {
        return "database URL has no database path"
    }
    $authority = $rest.Substring(0, $slash)
    $path = $rest.Substring($slash + 1)
    if ($path.Contains("/")) {
        return "database URL path is not a single database name"
    }
    if ([string]::IsNullOrEmpty($path)) {
        return "database URL has an empty database name"
    }
    try {
        $name = ConvertFrom-ColdStartPercentEncoding $path
    } catch {
        return "database name is not valid URL encoding"
    }
    if ($name.Length -gt 63 -or $name -notmatch '^fexu_test_[A-Za-z0-9_]+$') {
        return "database name must be an approved disposable name that starts with fexu_test_"
    }

    $hostPort = $authority
    $at = $authority.LastIndexOf("@")
    if ($at -ge 0) {
        $userInfo = $authority.Substring(0, $at)
        $hostPort = $authority.Substring($at + 1)
        if (-not [string]::IsNullOrEmpty($userInfo)) {
            try {
                $null = ConvertFrom-ColdStartPercentEncoding $userInfo
            } catch {
                return "database URL user info is not valid URL encoding"
            }
        }
    }
    if ([string]::IsNullOrEmpty($hostPort)) {
        return "database URL has no host"
    }

    # Match the host string PgConnectOptions stores. Do not percent-decode it.
    $hostName = ""
    $port = $null
    if ($hostPort -eq "[::1]") {
        $hostName = "[::1]"
    } elseif ($hostPort.StartsWith("[::1]:")) {
        $hostName = "[::1]"
        $port = $hostPort.Substring(6)
    } elseif ($hostPort.StartsWith("[")) {
        return "a disposable database name does not make this host a cold-start test instance; only a literal loopback host is accepted"
    } elseif ($hostPort.Contains("%")) {
        return "database URL host is not percent-decoded; only a literal loopback host is accepted, and a socket path is not a cold-start test instance"
    } else {
        $colon = $hostPort.IndexOf(":")
        if ($colon -ge 0) {
            if ($hostPort.IndexOf(":", $colon + 1) -ge 0) {
                return "database URL host is unclear"
            }
            $hostName = $hostPort.Substring(0, $colon)
            $port = $hostPort.Substring($colon + 1)
        } else {
            $hostName = $hostPort
        }
    }
    if ([string]::IsNullOrEmpty($hostName)) {
        return "database URL has no host"
    }
    if ($null -ne $port) {
        if ($port -notmatch '^[1-9][0-9]{0,4}$' -or [int]$port -gt 65535) {
            return "database URL port is unclear"
        }
    }
    $hostKey = $hostName.ToLowerInvariant()
    if ($hostKey -ne "127.0.0.1" -and $hostKey -ne "localhost" -and $hostKey -ne "[::1]") {
        return "a disposable database name does not make a remote PostgreSQL server a cold-start test instance; only a literal loopback host is accepted, and the URL user must already be a disposable test role"
    }
    return $null
}
