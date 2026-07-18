# s7s

Claude Code · Antigravity CLI · Codex의 대화 세션을 **하나의 TUI에서 통합 검색**하고,
선택한 세션의 프로젝트 폴더로 이동해 해당 에이전트 CLI로 **즉시 복귀(resume)** 하는
`k9s` 스타일 터미널 도구.

## 특징

- **통합 검색**: 세 도구에 흩어진 세션을 한 화면에서 조회.
- **고속 증분 캐시**: 파일 `mtime`만 훑어 변경분만 재파싱(`~/.cache/s7s/index.bin`).
- **클린 파서**: 날것 JSON 대신 **사용자 질문(User Turn)만** 정제 추출.
- **한글 NFC 정규화**: macOS 자소 분리(NFD) 문제로 인한 검색 누락 방지(초성 검색 제외).
- **양방향 라이프사이클**: TUI → 에이전트 CLI → TUI 복귀 시 필터 상태 유지.
- **사용량 표시**: 헤더에 프로필별 남은 사용량을 5h/주간 두 윈도우로
  표시(예: ` 72%(4h 30m)  52%(2d 16h) left`). 공식 CLI 화면(`/usage`·`/status`)을
  숨겨진 PTY에서 읽어 파싱한다 — [상세](docs/usage-display.md).
- **프로필(다중 구독)**: 에이전트 종류 + 이름 + config 폴더 + OAuth 토큰(저장만)을
  프로필로 관리. 같은 Claude라도 폴더가 다르면 별개 프로필(예: 개인/팀 구독)로 두고
  `CLAUDE_CONFIG_DIR`/`CODEX_HOME` 주입으로 세션 스캔·사용량 조회·resume을 해당
  구독으로 실행한다 — [상세](docs/profiles.md).

## 빌드 / 설치

### Via Homebrew (macOS)
```bash
brew tap ular-io/s7s
brew install s7s
```

### 직접 빌드 및 설치
```bash
cargo build --release
# 실행 파일: target/release/s7s
cp target/release/s7s ~/bin/   # 원하는 PATH 위치로 복사
```

## 사용법

```bash
s7s            # TUI 실행
s7s session <SESSION_ID>   # 과거 세션 컨텍스트 조회(TUI 없음, 아래 참조)
s7s --rebuild-cache  # 전체 캐시를 강제로 재생성
s7s --print    # 세션 목록만 출력(디버그/스크립트)
s7s --usage-probe    # TUI 없이 사용량 조회 결과만 출력(디버그)
s7s --help     # 도움말
s7s --version  # 버전
```

### 단축키 (세션 화면)

| 키 | 동작 |
| :-- | :-- |
| `:` | 화면 선택 메뉴 (`s` Session / `p` Profile) |
| `t` | 다음 화면으로 순환 전환(Session ↔ Profile) |
| `/` | 키워드 검색 모드(실시간 본문·제목 매칭, 공백=AND) |
| `a` | Agents 모달 (`space` 토글, `enter` 적용) |
| `1` ~ `9` | 활성 프로필 단독 필터(헤더 번호 순서) |
| `0` | 모든 필터 초기화 |
| `f` | Folder 모달 (타이핑=필터, `space` 토글, `enter` 적용) |
| `ctrl+u` | Update Session (세션 목록 추가/변경 반영 + 사용량 재조회) |
| `ctrl+n` | New Session (프로필/모델/폴더 대화상자) |
| `ctrl+shift+n` | New Session with Context (선택 세션을 과거 컨텍스트로 첨부, 아래 참조) |
| `ctrl+r` | Rename Session |
| `ctrl+d` | Delete Session 확인 |
| `tab` / `shift+tab` | 좌측 테이블 ↔ 우측 프리뷰 패널 포커스 토글 |
| `↑`/`↓` (`k`/`j`) | 테이블 포커스=행 이동 / 프리뷰 포커스=본문 스크롤 |
| `g` / `G` | 처음 / 끝으로 |
| `pageup` / `pagedown` | 프리뷰 본문 스크롤 |
| `enter` | Resume Session |
| `esc` | 검색/필터/선택 상태 취소(키워드·필터 초기화, 모달 닫기) — **종료 아님** |
| `q` / `ctrl+c` | 한 번 더 눌러 종료 |

모든 필터(키워드 · 에이전트 · 폴더 · 프로필)는 **AND 결합**으로 동작한다.

### 단축키 (프로필 화면, `:` → `p`)

| 키 | 동작 |
| :-- | :-- |
| `t` | 다음 화면으로 순환 전환(Profile → Session) |
| `enter` | 선택한 프로필로 새 세션 시작(폴더 직접 입력 또는 `↑`/`↓`로 기존 폴더 선택 후 실행, `tab`으로 full path 복사) |
| `space` | 프로필 활성화 토글(헤더 표시/번호 키 대상, 세션 목록은 전체 유지) |
| `+` | 프로필 추가 |
| `ctrl+e` | 프로필 편집 |
| `ctrl+d` | 프로필 삭제(기본 프로필 불가, 실제 폴더는 유지) |
| `ctrl+u` | 전체 프로필 사용량 갱신(갱신 중에도 직전 값 유지 표시) |
| `esc` | 세션 화면으로 복귀 |

## 세션 컨텍스트 (Session Context)

과거 세션의 대화 내용을 참조용 컨텍스트로 조회하거나, 선택한 세션을 컨텍스트로
붙여 새 세션을 시작할 수 있다 — [상세](docs/session-context.md).

### `s7s session` — 컨텍스트 조회 CLI

```bash
# 전체 활성 유저 턴 + 어시스턴트 발췌(과거 500자 / 마지막 턴 2,000자)
s7s session 019f36e8-9157-7c63-bee8-8937a6314982

# 유저 턴만
s7s session 019f36e8-9157-7c63-bee8-8937a6314982 --user-only

# 한 턴의 전체(redacted) 상세
s7s session 019f36e8-9157-7c63-bee8-8937a6314982 --turn 7

# 새 세션 초기화용(지침 봉투 포함)
s7s session 019f36e8-9157-7c63-bee8-8937a6314982 --agent codex --profile builtin-codex --bootstrap
```

- 기본 출력은 **중립적 참조 모드**: 신뢰 경계 문구만 있고 중지/대기/언어 지시가
  없어 진행 중인 다른 에이전트 세션 안에서 실행해도 안전하다.
- 전체 세션 ID를 전 프로필에서 정확 일치로 해석하며, 복수 매칭 시 후보를 나열하고
  `--agent`/`--profile` 지정을 요구한다. 시크릿은 발췌 전에 마스킹된다.
- 상세 옵션·발췌 한도·오류 규칙은 `s7s session --help` 참조.

### New Session with Context

Session/Detail 화면에서 `ctrl+shift+n`(또는 `:` 팔레트의 **New Session with
Context**)을 누르면 포커스된 세션이 **소스 세션**으로 캡처된 채 기존 New Session
대화상자가 열린다(타이틀에 소스 표시). Profile/Model/Folder는 평소처럼 자유롭게
선택 가능 — 소스와 다른 에이전트/프로젝트로도 시작할 수 있다. OK 시 새 에이전트에
짧은 부트스트랩 프롬프트가 주입되고, 새 에이전트는 `s7s session ... --bootstrap`으로
소스를 읽은 뒤 과거 작업을 수행하지 않고 소스 언어의 레디 메시지만 남기고 대기한다.
이후 첫 실제 요청(긴 텍스트·이미지 포함)은 에이전트 자체 UI에서 입력하면 된다.

> **터미널 호환성**: 레거시 터미널은 `Ctrl+Shift+N`과 `Ctrl+N`을 구분하지 못한다
> (동일 제어 바이트). s7s는 kitty 키보드 프로토콜을 지원하는 터미널에서만 chord를
> 구분하며, 그 외 환경에서는 `:` 팔레트의 **New Session with Context**가 보장된
> 폴백이다.

## 데이터 소스

| 에이전트 | 경로 | 세션 ID | resume |
| :-- | :-- | :-- | :-- |
| Claude | `~/.claude/projects/<enc>/<id>.jsonl` | 파일명 | `claude --resume <id>` |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | `session_meta.id` | `codex resume <id>` |
| Antigravity | `~/.gemini/antigravity-cli/history.jsonl` + `cache/conversation_metadata.json` | `conversationId` | `agy --conversation <id>` |

> Codex는 PRD의 `~/.config/codex/history/`가 아닌 실제 경로 `~/.codex/sessions/`,
> Antigravity는 `~/.config/antigravity/history/`가 아닌 `~/.gemini/antigravity-cli/`를 사용한다.

## 프로필 (다중 구독)

프로필 목록은 `~/.config/s7s/profiles.json`(macOS는
`~/Library/Application Support/s7s/`)에 앱이 저장하며, 최초 실행 시 기본
3개(Claude/Antigravity/Codex)를 시드한다. `:` → `p`의 프로필 화면에서 추가/편집한다.

- **path** = 에이전트 config 루트(예: `~/.claude-team`). 세션 디렉토리는 자동
  파생된다(Claude `<path>/projects`, Codex `<path>/sessions`, Antigravity는 path 자체).
- 기본 경로가 아닌 프로필만 `CLAUDE_CONFIG_DIR`/`CODEX_HOME`을 주입한다.
  기본 경로에 env를 명시하면 재로그인 화면이 뜨는 문제가 있다 — [상세](docs/profiles.md).
- 추가 프로필은 해당 config로 **한 번 직접 로그인 + 폴더 trust**를 마쳐야
  사용량 조회가 동작한다.

## 설정 (선택)

`~/.config/s7s/config.toml` 로 경로/`resume` 명령 템플릿을 덮어쓸 수 있다.
resume 템플릿 토큰: `{id}`(세션 ID), `{cwd}`(작업 폴더). 실행 시 `cd {cwd} && <템플릿>`
형태로 로그인 셸에서 동기 실행된다. 새 세션은 `new_*` 템플릿을 사용한다.

`:` 팔레트의 **Edit Config** 명령으로 이 파일을 열 수 있으며, 파일이 없으면
모든 키가 주석 처리된 템플릿(내장 기본값 표기)이 자동 생성된다. 주석을 푼
키만 기본값을 덮어쓴다.

```toml
resume_claude = "claude --resume {id}"
resume_codex = "codex resume {id}"
resume_antigravity = "agy --conversation {id}"
new_claude = "claude"
new_codex = "codex"
new_antigravity = "agy"
editor = "vim"
```

- **editor** = 기본 편집기 명령(선택). 설정하면 `!` Terminal Command 실행 셸에
  `EDITOR`/`VISUAL`로 export되어 `git commit` 등 편집기를 띄우는 명령에 적용된다.
  **Edit Config** 명령도 이 편집기를 사용하며(미설정 시 `$VISUAL` → `$EDITOR` →
  `vi` 순 폴백), 저장 후 s7s 복귀 시 설정이 즉시 재로드된다. 편집기 실행이
  실패하면(명령 오타 등) vim으로 다시 열지 확인한다.
  GUI 편집기는 종료까지 대기하는 플래그를 포함해야 한다(예: `code -w`).

> **Antigravity resume**: Antigravity CLI 실행 파일은 `agy`이며 `agy --conversation <id>`로
> 대화를 복귀한다. `agy`가 PATH에 없으면 절대 경로(예:
> `~/.local/bin/agy --conversation {id}`)로 `resume_antigravity`를 교체하라.

## 테마

`:` 팔레트의 **Change Theme** 명령으로 색 테마를 바꾼다. 대화상자는 한 번에
Dark 또는 Light 목록만 표시하며 ←/→로 목록 전체를 전환한다(상단 테두리의
좌우 화살표). ↑/↓ 이동 즉시 미리보기가 적용되고, enter로 확정(선택이
`~/.config/s7s/theme.json`에 저장), esc로 열기 전 테마로 복귀한다. 내장 테마는
40종(다크 20 · 라이트 20)이다.

- **기본 10종** — 다크: **Nord**(기본) · Tokyo Night · Dracula · Gruvbox Dark ·
  Solarized Dark · Catppuccin Mocha / 라이트: GitHub Light · Solarized Light ·
  Gruvbox Light · Catppuccin Latte.
- **인기 10종** — 다크: Monokai · One Dark · Night Owl · Ayu Dark ·
  Everforest Dark · Rosé Pine · Kanagawa / 라이트: One Light · Ayu Light ·
  Everforest Light.
- **다크 3종** — 내장 라이트 테마의 공식 다크 자매판: GitHub Dark ·
  Flexoki Dark · Tomorrow Night.
- **라이트 9종** — 내장 다크 테마의 공식 라이트 자매판 4종(Tokyo Night Day ·
  Rosé Pine Dawn · Kanagawa Lotus · Night Owl Light)과 인기 라이트 팔레트
  5종(Flexoki Light · Selenized Light · PaperColor Light · Tomorrow ·
  Modus Operandi).
- **Ular Dark · Ular Light** — Ular Light는 크림색 배경(`#FDF6E3`) 위에
  스틸블루 계열 잉크·액센트를 얹은 라이트 팔레트(모든 색이 hex 고정이라
  터미널 색 구성과 무관하게 동일하게 보임), Ular Dark는 브랜드 색(시안
  액센트, 에이전트 배지색)을 짙은 남회색 배경에 얹은 자체 다크 팔레트.
- **색약(CVD) 안전 6종** — 다크/라이트 각 3종(목록 마지막에 배치). 적록·청황
  색약에서도 심각도를 구분할 수 있도록 success/error를 초록↔빨강 대신
  파랑↔주황/버밀리언/마젠타로 매핑한다. 검증된 색맹 안전 팔레트 3종(Okabe-Ito ·
  IBM Carbon · Paul Tol) 기반: Okabe-Ito Dark/Light · IBM Carbon Dark/Light ·
  Paul Tol Dark/Light.

커스텀 테마는 `~/.config/s7s/themes/<키>.toml` 파일을 만들면 목록에 자동
포함된다. `base`(내장 테마 키, 기본 `nord`)에서 상속하고 `[colors]`에 지정한
롤만 덮어쓴다. 색 값은 `#RRGGBB` hex, ANSI 이름(`red`, `darkgray`, ...),
`default`(터미널 자체 색)를 지원한다.

```toml
name = "My Theme"
dark = true
base = "nord"

[colors]
bg = "default"        # keep the terminal's own background
accent = "#88C0D0"    # focus borders / selection
```

## 아키텍처

```
main.rs         진입점(clap) · 터미널 라이프사이클 · 이벤트 루프 · resume 핸드오버
config.rs       경로 · resume/new 명령 템플릿({prompt} 토큰) · 캐시/설정 위치
profile.rs      프로필(다중 구독) 모델 · profiles.json 로드/저장/시드 · env 매핑
model.rs        Session/Agent 타입 · 날짜 포맷
normalize.rs    NFC 정규화
parser/         claude.rs · codex.rs · antigravity.rs (유저턴만 추출)
session_context/ 공유 세션 컨텍스트(상세 파서·발췌·마스킹·해석·렌더링)
session_cli.rs  `s7s session` 서브커맨드 실행
handoff.rs      HandoffTurn 호환 어댑터 + Markdown 익스포터
cache.rs        mtime 기반 bincode 캐시
scan.rs         프로필 단위 증분 스캐너
filter.rs       키워드·에이전트·폴더·프로필 AND 복합 필터
theme.rs        색 테마(내장 40종 · themes/*.toml 커스텀 · theme.json 선택 저장)
ui/mod.rs       App 상태 · Screen/UiMode 상태머신 · 키 입력 처리
ui/render.rs    헤더 · 세션/프로필 테이블 · 프리뷰 · 상태바 · 모달 렌더링
usage.rs        프로필별 CLI 사용량 조회(PTY 구동 · env 주입 · 화면 파싱)
```

## 문서

- [AGENTS.md](./AGENTS.md)
- [Panel Focus Style](./docs/panel-focus-style.md)
- [Preview Omission Style](./docs/preview-omission-style.md)
- [Session Title Compatibility](./docs/session-title-compat.md)
- [Testing Guide](./docs/testing.md)
- [Usage Display](./docs/usage-display.md)
- [Profiles](./docs/profiles.md)
- [Session Context](./docs/session-context.md)

