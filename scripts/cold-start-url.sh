# Shared PostgreSQL guard for the embedded cold start.
# A disposable name is not enough: the host must be loopback, and the URL user
# must already be a disposable test role. User, password, host, and query never
# satisfy the database-name rule. Unsupported or unclear URLs are refused.

url_decode() {
  local raw="$1"
  local i=0
  local len=${#raw}
  local out=""
  while [ "$i" -lt "$len" ]; do
    local c="${raw:$i:1}"
    if [ "$c" = "%" ]; then
      if [ $((i + 2)) -ge "$len" ]; then
        return 1
      fi
      local hex="${raw:$((i + 1)):2}"
      case "$hex" in
        [0-9A-Fa-f][0-9A-Fa-f]) ;;
        *) return 1 ;;
      esac
      local hexlower
      hexlower=$(printf '%s' "$hex" | tr 'A-F' 'a-f')
      if [ "$hexlower" = "00" ]; then
        return 1
      fi
      local decoded
      decoded=$(printf '%b' "\\x${hex}")
      out="${out}${decoded}"
      i=$((i + 3))
    else
      out="${out}${c}"
      i=$((i + 1))
    fi
  done
  printf '%s' "$out"
}

cold_start_database_rejection() {
  local url="$1"
  case "$url" in
    *[[:space:]]*|*"#"*)
      echo "database URL is unclear"
      return 0
      ;;
  esac
  case "$url" in
    *"://"*) ;;
    *)
      echo "unsupported or unclear database URL"
      return 0
      ;;
  esac
  local scheme="${url%%://*}"
  scheme=$(printf '%s' "$scheme" | tr '[:upper:]' '[:lower:]')
  case "$scheme" in
    sqlite) return 0 ;;
    postgres|postgresql) ;;
    *)
      echo "unsupported database URL scheme"
      return 0
      ;;
  esac

  local rest="${url#*://}"
  rest="${rest%%\?*}"
  case "$rest" in
    */*) ;;
    *)
      echo "database URL has no database path"
      return 0
      ;;
  esac
  local authority="${rest%%/*}"
  local path="${rest#*/}"
  case "$path" in
    */*)
      echo "database URL path is not a single database name"
      return 0
      ;;
    "")
      echo "database URL has an empty database name"
      return 0
      ;;
  esac
  local name
  if ! name=$(url_decode "$path"); then
    echo "database name is not valid URL encoding"
    return 0
  fi
  case "$name" in
    fexu_test_*) ;;
    *)
      echo "database name must be an approved disposable name that starts with fexu_test_"
      return 0
      ;;
  esac
  local suffix="${name#fexu_test_}"
  if [ -z "$suffix" ]; then
    echo "database name must be an approved disposable name that starts with fexu_test_"
    return 0
  fi
  case "$suffix" in
    *[!A-Za-z0-9_]*)
      echo "database name must be an approved disposable name that starts with fexu_test_"
      return 0
      ;;
  esac
  if [ "${#name}" -gt 63 ]; then
    echo "database name must be an approved disposable name that starts with fexu_test_"
    return 0
  fi

  local hostport="$authority"
  case "$hostport" in
    *@*) hostport="${hostport##*@}" ;;
  esac
  if [ -z "$hostport" ]; then
    echo "database URL has no host"
    return 0
  fi
  local host=""
  case "$hostport" in
    \[*\])
      host="${hostport#\[}"
      host="${host%%\]*}"
      ;;
    \[*:*)
      host="${hostport#\[}"
      host="${host%%\]*}"
      local after=":${hostport#*\]:}"
      case "$after" in
        :*[!0-9]*)
          echo "database URL port is unclear"
          return 0
          ;;
        :)
          echo "database URL port is unclear"
          return 0
          ;;
      esac
      ;;
    *:*:*)
      echo "database URL host is unclear"
      return 0
      ;;
    *:*)
      host="${hostport%%:*}"
      local port="${hostport#*:}"
      case "$port" in
        ""|*[!0-9]*)
          echo "database URL port is unclear"
          return 0
          ;;
      esac
      ;;
    *) host="$hostport" ;;
  esac
  host=$(printf '%s' "$host" | tr '[:upper:]' '[:lower:]')
  case "$host" in
    127.0.0.1|localhost|::1) ;;
    *)
      echo "a disposable database name does not make a remote PostgreSQL server a cold-start test instance; only a loopback host is accepted, and the URL user must already be a disposable test role"
      return 0
      ;;
  esac
}
