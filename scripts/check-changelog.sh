#!/bin/sh
# Checks a changelog against Keep a Changelog 1.1.0.
#
# Usage: scripts/check-changelog.sh <path>
#
# Rules: "# Changelog" title; "## [Unreleased]" as the first section; every
# later section "## [X.Y.Z] - YYYY-MM-DD", newest first; only the six
# standard "###" groups, each at most once per section; a link reference for
# every section.

set -eu

file=${1:?usage: scripts/check-changelog.sh <path>}

awk '
function fail(message) {
    printf "CHANGELOG.md:%d: %s\n", FNR, message > "/dev/stderr"
    failed = 1
}

# 1 when version a is newer than version b, 0 when equal, -1 when older.
function compare(a, b,    ca, cb, pa, pb, na, nb, i) {
    ca = a; pa = ""; if (index(a, "-")) { ca = substr(a, 1, index(a, "-") - 1); pa = substr(a, index(a, "-") + 1) }
    cb = b; pb = ""; if (index(b, "-")) { cb = substr(b, 1, index(b, "-") - 1); pb = substr(b, index(b, "-") + 1) }
    split(ca, na, "."); split(cb, nb, ".")
    for (i = 1; i <= 3; i++) {
        if (na[i] + 0 > nb[i] + 0) return 1
        if (na[i] + 0 < nb[i] + 0) return -1
    }
    if (pa == pb) return 0
    if (pa == "") return 1
    if (pb == "") return -1
    return pa > pb ? 1 : -1
}

BEGIN {
    split("Added Changed Deprecated Removed Fixed Security", names, " ")
    for (i in names) groups[names[i]] = 1
}

FNR == 1 && $0 != "# Changelog" { fail("the first line must be \"# Changelog\"") }

/^## / {
    sections++
    split("", seen)

    if (sections == 1) {
        if ($0 == "## [Unreleased]") labels[sections] = "unreleased"
        else fail("the first section must be \"## [Unreleased]\", got \"" $0 "\"")
        next
    }

    if ($0 !~ /^## \[[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?\] - [0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]$/) {
        fail("a release heading must be \"## [X.Y.Z] - YYYY-MM-DD\", got \"" $0 "\"")
        next
    }

    version = substr($0, 5, index($0, "]") - 5)
    date = substr($0, length($0) - 9)
    labels[sections] = version

    if (previous_version != "" && compare(version, previous_version) >= 0)
        fail("[" version "] must be older than [" previous_version "] above it")
    if (previous_date != "" && date > previous_date)
        fail("[" version "] is dated " date ", after [" previous_version "] above it")

    previous_version = version
    previous_date = date
    next
}

/^### / {
    group = substr($0, 5)
    if (sections == 0) fail("\"" $0 "\" must belong to a \"##\" section")
    if (!(group in groups)) fail("\"" group "\" is not one of Added, Changed, Deprecated, Removed, Fixed, Security")
    if (group in seen) fail("\"" group "\" appears twice in one section")
    seen[group] = 1
}

/^\[[^]]+\]: [^ ]+$/ {
    links[tolower(substr($0, 2, index($0, "]") - 2))] = 1
}

END {
    if (sections == 0) {
        fail("no \"## [Unreleased]\" section")
    }
    for (i = 1; i <= sections; i++) {
        if (labels[i] != "" && !(labels[i] in links)) {
            printf "CHANGELOG.md: no link reference \"[%s]: <url>\" at the end\n", labels[i] > "/dev/stderr"
            failed = 1
        }
    }
    exit failed
}
' "$file"
