#!/usr/bin/env bash
# Gerüst der lokalen CI von remotehub. Ursprünglich aus einem anderen Repo
# kopiert; es gehört jetzt allein diesem Repo und wird hier frei geändert.
# Nichts davon wird mit anderen Repos geteilt oder abgeglichen.
#
# Warum lokal: Private Repos haben 3 000 Action-Minuten im Monat, die im
# September 2026 nach zwei Wochen aufgebraucht waren. Deshalb laufen alle
# Prüfungen auf dem eigenen Rechner und melden ihr Ergebnis als Commit-Status
# „lokal" an GitHub. Das Ruleset verlangt diesen Status vor dem Merge.
#
# Das repo-eigene scripts/ci/lokal.sh legt CI_REPO_KURZ, CI_JOBS und je Job
# eine Funktion job_<name> an, bindet diese Datei ein und ruft ci_main "$@".
#
# Geprüft wird immer ein Commit, nie die Arbeitskopie: Der Commit wird per
# `git archive` in ein Docker-Volume ausgepackt (LF-Zeilenenden wie auf Linux).
# Jobs laufen in Containern, die das Volume unter seinem Mountpoint sehen —
# dadurch stimmen Pfade auch für `docker compose` und Bind-Mounts, die ein Job
# über den Docker-Socket selbst startet.
#
# Voraussetzungen auf dem Rechner: bash (unter Windows Git Bash), git, docker,
# gh (angemeldet, Scope repo).

set -euo pipefail

# Git Bash unter Windows schreibt sonst /var/... in C:/Program Files/Git/var/... um.
export MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*'

CI_KONTEXT="lokal"

ci_fehler() {
  printf '\n✗ %s\n' "$*" >&2
  exit 1
}

ci_hilfe() {
  cat <<EOF
Lokale CI für ${CI_REPO_KURZ} — ersetzt GitHub Actions.

  bash scripts/ci/lokal.sh              prüft HEAD und meldet den Status „${CI_KONTEXT}"
  bash scripts/ci/lokal.sh --pr 12      prüft den Kopf-Commit von Pull Request #12
  bash scripts/ci/lokal.sh --ref REF    prüft einen beliebigen Commit
  bash scripts/ci/lokal.sh --nur a,b    nur diese Jobs (meldet keinen Status)
  bash scripts/ci/lokal.sh --melden     meldet ein früheres Ergebnis für HEAD nach
                                        (etwa wenn vor dem Push geprüft wurde)
  bash scripts/ci/lokal.sh --ohne-status  prüft nur, meldet nichts
  bash scripts/ci/lokal.sh --laut       Job-Ausgaben mitschreiben statt nur ins Protokoll

Jobs: ${CI_JOBS[*]}
Je Repo läuft immer nur eine lokale CI; weitere Läufe dieses Repos warten.
Ein Commit mit demselben Dateistand wie ein schon grüner wird nicht erneut
geprüft (CI_FULL=1 erzwingt einen Lauf).
EOF
}

# ── Aufruf ────────────────────────────────────────────────────────────────────

CI_ZIEL="HEAD"
CI_PR=""
CI_NUR=""
CI_STATUS=1
CI_NUR_MELDEN=0
CI_LAUT=0

ci_argumente() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --pr) CI_PR="${2:?--pr braucht eine Nummer}"; shift 2 ;;
      --ref) CI_ZIEL="${2:?--ref braucht einen Commit}"; shift 2 ;;
      --nur) CI_NUR="${2:?--nur braucht Jobnamen}"; shift 2 ;;
      --ohne-status) CI_STATUS=0; shift ;;
      --melden) CI_NUR_MELDEN=1; shift ;;
      --laut) CI_LAUT=1; shift ;;
      -h | --help) ci_hilfe; exit 0 ;;
      *) ci_fehler "Unbekanntes Argument: $1 (siehe --help)" ;;
    esac
  done
}

ci_dauer() {
  local s="$1"
  if [ "$s" -ge 60 ]; then printf '%dm%02ds' $((s / 60)) $((s % 60)); else printf '%ds' "$s"; fi
}

# ── Commit, Repo, Ergebnisablage ──────────────────────────────────────────────

ci_ziel_bestimmen() {
  command -v git >/dev/null || ci_fehler "git fehlt"
  command -v docker >/dev/null || ci_fehler "docker fehlt"
  docker info >/dev/null 2>&1 || ci_fehler "Docker läuft nicht"

  CI_WURZEL="$(git rev-parse --show-toplevel)"
  if [ -n "$CI_PR" ]; then
    git -C "$CI_WURZEL" fetch -q origin "pull/${CI_PR}/head" ||
      ci_fehler "Pull Request #${CI_PR} nicht gefunden"
    CI_SHA="$(git -C "$CI_WURZEL" rev-parse FETCH_HEAD)"
  else
    CI_SHA="$(git -C "$CI_WURZEL" rev-parse --verify "${CI_ZIEL}^{commit}")" ||
      ci_fehler "Commit ${CI_ZIEL} unbekannt"
    if [ "$CI_ZIEL" = "HEAD" ] && [ -n "$(git -C "$CI_WURZEL" status --porcelain --untracked-files=no)" ]; then
      echo "! Nicht committete Änderungen werden nicht geprüft — geprüft wird HEAD."
    fi
  fi
  CI_SHA_KURZ="${CI_SHA:0:7}"

  CI_ABLAGE="$(git -C "$CI_WURZEL" rev-parse --path-format=absolute --git-common-dir)/ci-lokal"
  mkdir -p "$CI_ABLAGE"

  CI_GITHUB=""
  if [ "$CI_STATUS" = 1 ]; then
    command -v gh >/dev/null || ci_fehler "gh fehlt (oder --ohne-status)"
    CI_GITHUB="$(cd "$CI_WURZEL" && gh repo view --json nameWithOwner -q .nameWithOwner)" ||
      ci_fehler "gh kennt das Repo nicht (angemeldet?)"
  fi
}

ci_auf_github() {
  gh api "repos/${CI_GITHUB}/commits/${CI_SHA}" --silent >/dev/null 2>&1
}

ci_status_setzen() { # state beschreibung
  local text="$2"
  [ "${#text}" -le 140 ] || text="${text:0:137}..."
  gh api -X POST "repos/${CI_GITHUB}/statuses/${CI_SHA}" \
    -f state="$1" -f context="$CI_KONTEXT" -f description="$text" >/dev/null
}

ci_melden() {
  local datei="${CI_ABLAGE}/${CI_SHA}"
  [ -f "$datei" ] || ci_fehler "Für ${CI_SHA_KURZ} liegt kein lokales Ergebnis vor — erst prüfen."
  local ergebnis beschreibung
  ergebnis="$(head -1 "$datei")"
  beschreibung="$(sed -n 2p "$datei")"
  [ "$ergebnis" = "ok" ] || ci_fehler "Die lokale Prüfung von ${CI_SHA_KURZ} war nicht grün."
  ci_auf_github || ci_fehler "Commit ${CI_SHA_KURZ} ist noch nicht auf GitHub — erst pushen."
  ci_status_setzen success "$beschreibung"
  echo "✓ Status „${CI_KONTEXT}\" für ${CI_SHA_KURZ} gemeldet: ${beschreibung}"
}

# ── Sperre je Repo ────────────────────────────────────────────────────────────
# Zwei Läufe dieses Repos teilen Cache-Volumes und den Compose-Projektnamen;
# der zweite wartet. Andere Repos sind davon völlig unabhängig. mkdir ist
# atomar; ein Verzeichnis ohne lebenden Inhaber gilt als verwaist.

CI_SPERRE=""

ci_sperren() {
  local sperre="${CI_ABLAGE}/sperre" gemeldet=0 inhaber pid
  until mkdir "$sperre" 2>/dev/null; do
    inhaber="$(cat "${sperre}/inhaber" 2>/dev/null || true)"
    pid="${inhaber%% *}"
    if [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null; then
      rm -rf "$sperre"
      continue
    fi
    if [ "$gemeldet" = 0 ]; then
      echo "… warte auf einen anderen CI-Lauf dieses Repos: ${inhaber#* }"
      gemeldet=1
    fi
    sleep 5
  done
  CI_SPERRE="$sperre"
  echo "$$ ${CI_REPO_KURZ} ${CI_SHA_KURZ} seit $(date +%H:%M)" >"${sperre}/inhaber"
}

# ── Arbeitsumgebung ───────────────────────────────────────────────────────────

CI_ID=""
CI_VOLUME=""
CI_NETZ=""
CI_SRC=""
CI_PENDING=0
# Fester Compose-Projektname je Repo: Die Sperre verhindert Überschneidungen,
# und Images, die Compose baut, werden beim nächsten Lauf wiederverwendet.
CI_COMPOSE=""

ci_aufraeumen() {
  local rc=$?
  set +e
  if [ -n "$CI_ID" ]; then
    # Eigene Container und alles, was Jobs per `docker compose` selbst gestartet
    # haben (Projektname CI_COMPOSE). Images bleiben als Cache stehen.
    local filter
    for filter in "label=ci-lokal=${CI_ID}" "label=com.docker.compose.project=${CI_COMPOSE}"; do
      # shellcheck disable=SC2046
      docker rm -f -v $(docker ps -aq --filter "$filter") >/dev/null 2>&1
      # shellcheck disable=SC2046
      docker network rm $(docker network ls -q --filter "$filter") >/dev/null 2>&1
      # shellcheck disable=SC2046
      docker volume rm $(docker volume ls -q --filter "$filter") >/dev/null 2>&1
    done
  fi
  if [ "$CI_PENDING" = 1 ] && [ "$rc" -ne 0 ]; then
    ci_status_setzen error "Lokaler Lauf abgebrochen" 2>/dev/null
  fi
  [ -z "$CI_SPERRE" ] || rm -rf "$CI_SPERRE"
  exit "$rc"
}

ci_umgebung() {
  CI_ID="ci-${CI_REPO_KURZ}-${CI_SHA_KURZ}-$(date +%H%M%S)-$$"
  CI_ID="$(printf '%s' "$CI_ID" | tr '[:upper:].' '[:lower:]-')"
  CI_VOLUME="$CI_ID"
  CI_NETZ="$CI_ID"
  CI_COMPOSE="$(printf 'ci-%s' "$CI_REPO_KURZ" | tr '[:upper:].' '[:lower:]-')"
  docker volume create --label "ci-lokal=${CI_ID}" "$CI_VOLUME" >/dev/null
  docker network create --label "ci-lokal=${CI_ID}" "$CI_NETZ" >/dev/null
  CI_SRC="$(docker volume inspect -f '{{.Mountpoint}}' "$CI_VOLUME")"

  # Linux-Zeilenenden erzwingen, egal was core.autocrlf/core.eol lokal sagen.
  git -C "$CI_WURZEL" -c core.autocrlf=false -c core.eol=lf archive --format=tar "$CI_SHA" |
    docker run --rm -i --quiet --label "ci-lokal=${CI_ID}" -v "${CI_VOLUME}:/src" alpine:3 tar -x -C /src
}

# Container im Netz des Laufs, Quelltext unter $CI_SRC, Docker-Socket dabei.
#   ci_docker_run [docker-run-Optionen] IMAGE [BEFEHL...]
ci_docker_run() {
  docker run --rm --label "ci-lokal=${CI_ID}" --network "$CI_NETZ" \
    -v /var/run/docker.sock:/var/run/docker.sock \
    -v "${CI_VOLUME}:${CI_SRC}" -w "$CI_SRC" \
    -e CI=true -e CI_LOKAL=1 -e "COMPOSE_PROJECT_NAME=${CI_COMPOSE}" \
    "$@"
}

# Hintergrunddienst im Netz des Laufs (wie `services:` im Workflow).
#   ci_dienst NAME [docker-run-Optionen] IMAGE [BEFEHL...]
ci_dienst() {
  local name="$1"
  shift
  docker run -d --label "ci-lokal=${CI_ID}" --network "$CI_NETZ" \
    --network-alias "$name" --name "${CI_ID}-${name}" "$@" >/dev/null
}

# Wartet, bis ein Befehl im Dienst-Container gelingt.
#   ci_warten NAME SEKUNDEN BEFEHL...
ci_warten() {
  local name="$1" frist="$2"
  shift 2
  local ende=$((SECONDS + frist))
  until docker exec "${CI_ID}-${name}" "$@" >/dev/null 2>&1; do
    [ "$SECONDS" -lt "$ende" ] || {
      docker logs --tail 50 "${CI_ID}-${name}"
      echo "Dienst ${name} wurde nicht bereit"
      return 1
    }
    sleep 1
  done
}

# Werkzeug-Image aus einem Dockerfile der Arbeitskopie, getaggt nach Inhalt.
#   ci_image DOCKERFILE → gibt den Tag aus
ci_image() {
  local datei="$1" tag
  tag="${CI_REPO_KURZ}-ci-werkzeug:$(git -C "$CI_WURZEL" hash-object "${CI_WURZEL}/${datei}" | cut -c1-12)"
  tag="$(printf '%s' "$tag" | tr '[:upper:]' '[:lower:]')"
  if ! docker image inspect "$tag" >/dev/null 2>&1; then
    # Fortschritt ins Job-Protokoll (stderr), nur der Tag auf stdout.
    docker build --progress=plain -t "$tag" - <"${CI_WURZEL}/${datei}" >&2
  fi
  printf '%s' "$tag"
}

# ── Nichts doppelt prüfen ─────────────────────────────────────────────────────
# Ein Commit mit demselben Dateistand (Git-Tree) wie ein schon grün geprüfter,
# etwa der Merge-Commit eines PRs, übernimmt dessen Ergebnis statt neu zu laufen.

ci_baum() {
  git -C "$CI_WURZEL" rev-parse "${CI_SHA}^{tree}"
}

ci_schon_gruen() {
  local datei quelle beschreibung
  datei="${CI_ABLAGE}/baum-$(ci_baum)"
  [ -f "$datei" ] || return 1
  quelle="$(sed -n 1p "$datei")"
  beschreibung="gleicher Stand wie ${quelle:0:7} · $(sed -n 2p "$datei")"
  [ -n "$quelle" ] || return 1
  printf 'ok\n%s\n' "$beschreibung" >"${CI_ABLAGE}/${CI_SHA}"
  if [ "$CI_STATUS" = 1 ]; then
    if ci_auf_github; then
      ci_status_setzen success "$beschreibung"
      echo "✓ ${CI_SHA_KURZ}: ${beschreibung} — Status „${CI_KONTEXT}\" gemeldet, nicht erneut geprüft"
      return 0
    fi
    echo "  Commit ist noch nicht auf GitHub — Status danach mit --melden nachtragen."
  fi
  echo "✓ ${CI_SHA_KURZ}: ${beschreibung} — nicht erneut geprüft"
}

# ── Ablauf ────────────────────────────────────────────────────────────────────

ci_main() {
  ci_argumente "$@"
  ci_ziel_bestimmen

  if [ "$CI_NUR_MELDEN" = 1 ]; then
    ci_melden
    return
  fi

  if [ -z "$CI_NUR" ] && [ "${CI_FULL:-0}" != 1 ] && ci_schon_gruen; then
    return
  fi

  local jobs=("${CI_JOBS[@]}")
  if [ -n "$CI_NUR" ]; then
    IFS=',' read -r -a jobs <<<"$CI_NUR"
    CI_STATUS=0
    for j in "${jobs[@]}"; do
      declare -F "job_${j}" >/dev/null || ci_fehler "Unbekannter Job: ${j} (vorhanden: ${CI_JOBS[*]})"
    done
  fi

  trap ci_aufraeumen EXIT
  trap 'exit 130' INT TERM

  ci_sperren

  local protokolle
  protokolle="${CI_ABLAGE}/protokolle/${CI_SHA_KURZ}-$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$protokolle"

  echo "▶ ${CI_REPO_KURZ} ${CI_SHA_KURZ} — $(git -C "$CI_WURZEL" log -1 --format=%s "$CI_SHA")"
  echo "  Protokolle: ${protokolle}"

  if [ "$CI_STATUS" = 1 ]; then
    if ci_auf_github; then
      ci_status_setzen pending "Läuft lokal auf $(hostname)"
      CI_PENDING=1
    else
      echo "  Commit ist noch nicht auf GitHub — Status danach mit --melden nachtragen."
    fi
  fi

  ci_umgebung

  local start=$SECONDS gruen=() rot=() j t rc
  for j in "${jobs[@]}"; do
    t=$SECONDS
    printf '  • %-12s ' "$j"
    set +e
    if [ "$CI_LAUT" = 1 ]; then
      echo
      (set -euo pipefail; "job_${j}") 2>&1 | tee "${protokolle}/${j}.log"
      rc=${PIPESTATUS[0]}
    else
      (set -euo pipefail; "job_${j}") >"${protokolle}/${j}.log" 2>&1
      rc=$?
    fi
    set -e
    if [ "$rc" -eq 0 ]; then
      gruen+=("$j")
      echo "✓ $(ci_dauer $((SECONDS - t)))"
    else
      rot+=("$j")
      echo "✗ $(ci_dauer $((SECONDS - t))) (Exit ${rc})"
      if [ "$CI_LAUT" = 0 ]; then
        echo "  ── letzte Zeilen aus ${j}.log ──"
        tail -n 40 "${protokolle}/${j}.log" | sed 's/^/    /'
      fi
    fi
  done

  local dauer beschreibung
  dauer="$(ci_dauer $((SECONDS - start)))"
  if [ "${#rot[@]}" -eq 0 ]; then
    beschreibung="${gruen[*]} grün · ${dauer} · $(hostname)"
    if [ -z "$CI_NUR" ]; then
      printf 'ok\n%s\n' "$beschreibung" >"${CI_ABLAGE}/${CI_SHA}"
      printf '%s\n%s\n' "$CI_SHA" "$beschreibung" >"${CI_ABLAGE}/baum-$(ci_baum)"
    fi
    if [ "$CI_STATUS" = 1 ] && [ "$CI_PENDING" = 1 ]; then
      ci_status_setzen success "$beschreibung"
      CI_PENDING=0
      echo "✓ ${beschreibung} — Status „${CI_KONTEXT}\" gemeldet"
    else
      echo "✓ ${beschreibung}"
    fi
    return 0
  fi

  beschreibung="rot: ${rot[*]} · ${dauer} · $(hostname)"
  printf 'fehler\n%s\n' "$beschreibung" >"${CI_ABLAGE}/${CI_SHA}"
  if [ "$CI_PENDING" = 1 ]; then
    ci_status_setzen failure "$beschreibung"
    CI_PENDING=0
  fi
  echo "✗ ${beschreibung}"
  return 1
}
