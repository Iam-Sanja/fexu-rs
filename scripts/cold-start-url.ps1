# Shared PostgreSQL guard for the embedded cold start.
# A disposable name is not enough: the host must be loopback, and the URL user
# must already be a disposable test role. The scripts do not read pg_roles and
# do not verify that role's privileges. Any query string is refused, including
# an empty query and parameters such as dbname, host, hostaddr, port, user,
# password, options, and application_name. The query is not removed before this
# decision. User, password, and host never satisfy the database-name rule.
# Unsupported or unclear URLs are refused.

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
    $at = $hostPort.LastIndexOf("@")
    if ($at -ge 0) {
        $hostPort = $hostPort.Substring($at + 1)
    }
    if ([string]::IsNullOrEmpty($hostPort)) {
        return "database URL has no host"
    }
    $hostName = ""
    if ($hostPort.StartsWith("[")) {
        $end = $hostPort.IndexOf("]")
        if ($end -lt 2) {
            return "database URL host is unclear"
        }
        $hostName = $hostPort.Substring(1, $end - 1)
        $after = $hostPort.Substring($end + 1)
        if ($after.Length -gt 0 -and $after -notmatch '^:[0-9]+$') {
            return "database URL port is unclear"
        }
    } else {
        $colon = $hostPort.IndexOf(":")
        if ($colon -ge 0) {
            if ($hostPort.IndexOf(":", $colon + 1) -ge 0) {
                return "database URL host is unclear"
            }
            $hostName = $hostPort.Substring(0, $colon)
            $port = $hostPort.Substring($colon + 1)
            if ($port -notmatch '^[0-9]+$') {
                return "database URL port is unclear"
            }
        } else {
            $hostName = $hostPort
        }
    }
    try {
        $hostName = ConvertFrom-ColdStartPercentEncoding $hostName
    } catch {
        return "database URL host is not valid URL encoding"
    }
    $hostName = $hostName.ToLowerInvariant()
    if ($hostName.StartsWith("/")) {
        return "a disposable database name does not make a socket path a cold-start test instance; only a loopback host is accepted, and the URL user must already be a disposable test role"
    }
    if ($hostName -ne "127.0.0.1" -and $hostName -ne "localhost" -and $hostName -ne "::1") {
        return "a disposable database name does not make a remote PostgreSQL server a cold-start test instance; only a loopback host is accepted, and the URL user must already be a disposable test role"
    }
    return $null
}
