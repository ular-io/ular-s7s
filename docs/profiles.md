# Profiles (다중 구독)

프로필 = **에이전트 종류 + 이름 + config 폴더(path) + OAuth 토큰(저장만)** 의 묶음.
같은 에이전트라도 config 폴더가 다르면 별개 프로필이다(예: Claude 개인 구독 +
팀 구독). 구현은 `src/profile.rs`(모델·저장), `src/ui/mod.rs`(프로필 화면/폼).

## 저장 위치

- `~/.config/s7s/profiles.json` (`config_base_dir()`, `src/config.rs` — hardcoded, all platforms)
- 앱이 소유·저장하는 파일이다(설정용 `config.toml`은 사용자 수동 편집, 프로필과 별개).
- OAuth 토큰이 평문으로 들어갈 수 있어 저장 시 **0600 권한**을 준다.
- 최초 실행 시 기본 3개(builtin)를 시드한다. builtin은 삭제 불가·에이전트 변경 불가이며,
  수동 편집으로 지워도 로드 시 재시드된다.

## path의 의미

프로필 path = 에이전트 **config 루트**. 세션 디렉토리는 파생 규칙으로 얻는다.

| 에이전트 | 기본 루트 | 세션 디렉토리 |
| :-- | :-- | :-- |
| Claude | `~/.claude` | `<path>/projects` |
| Codex | `~/.codex` | `<path>/sessions` |
| Antigravity | `~/.gemini/antigravity-cli` | `<path>` 자체 |

기본 프로필의 path는 `config.toml`의 디렉토리 오버라이드를 흡수해 시드된다
(세션 디렉토리의 basename이 파생 규칙과 일치하면 parent를 루트로 사용).

## env 주입 규칙 (핵심 주의사항)

usage 조회와 resume/새 세션 실행, 그리고 Claude rename의 CLI 시도
(`claude --resume <id> --name ...`) 시 프로필 path를 환경변수로 주입한다:

| 에이전트 | 환경변수 | 비고 |
| :-- | :-- | :-- |
| Claude | `CLAUDE_CONFIG_DIR` | |
| Codex | `CODEX_HOME` | |
| Antigravity | 없음 | 추가 프로필은 usage 스킵, resume은 기본 계정으로 실행 |

Antigravity는 전용 변수가 없음을 실측으로 확정했다(2026-07-14, agy 1.1.2 —
바이너리 strings 전수 조사 + `ANTIGRAVITY_CONFIG_DIR` 지정 부팅 실험,
[Model Selection](models.md)의 "agy env 주입 검증" 절). 이에 따라 **Add/Edit
Profile 폼에서 Antigravity는 dim + 선택 불가**다(기존 Antigravity 프로필의
편집·삭제만 허용, `ProfileFormState::agy_allowed`) — 만들 수는 있지만 아무
기능도 없는 추가 프로필의 생성을 원천 차단한다.

**기본 경로 프로필에는 주입하지 않는다** (`Profile::env_var()`가 None 반환).
실측으로 확인한 이유:

- Claude Code의 핵심 상태 파일 `.claude.json`(온보딩·계정·폴더 trust)은
  env 미설정 시 **`~/.claude.json`(홈 루트)**, `CLAUDE_CONFIG_DIR` 설정 시
  **`<dir>/.claude.json`** 에 위치한다.
- 따라서 `CLAUDE_CONFIG_DIR=~/.claude claude`는 `claude`와 **동일하지 않다** —
  `~/.claude/.claude.json`이 없으면 신규 설치로 인식해 테마 선택/로그인부터 다시 시작한다.
- 키체인 인증 항목도 config 경로 기준으로 분리된다.

### 오염 env 정리 (transcript 저장 보장)

Claude Code 세션은 자식 프로세스에 `CLAUDECODE=1`, `CLAUDE_CODE_SESSION_ID`,
`CLAUDE_CODE_CHILD_SESSION=1` 등을 주입한다. s7s가 claude 세션 내부(`!`/Bash)에서
실행되면 이 변수들을 물려받고, s7s가 띄운 claude(2.1.204+에서 실측 확인)는
`CLAUDE_CODE_CHILD_SESSION=1`을 보고 자신을 자동화용 자식 세션으로 간주해
**transcript 저장을 통째로 건너뛴다** — 세션이 s7s 목록과 `/resume` 양쪽에서
사라진 것처럼 보인다(실제로는 저장 자체가 안 된 것, 복구 불가).

이를 막기 위해 `src/resume.rs::sanitize_agent_env()`가 resume/새 세션 spawn 시
프로세스 env에서 위 변수들을 제거하고 `CLAUDE_CODE_FORCE_SESSION_PERSISTENCE=1`을
주입한다. 프로필 env(`CLAUDE_CONFIG_DIR` 등)와 달리 명령 문자열 접두가 아닌
프로세스 env로 처리한다 — 셸 초기화가 이 변수들을 재설정할 일이 없고 미리보기
문자열을 어지럽히지 않기 위함이다.

## 추가 프로필 초기 설정

새 config 폴더는 로그인/trust 상태가 비어 있으므로 한 번 직접 실행해 마쳐야 한다:

```bash
CLAUDE_CONFIG_DIR=~/.claude-team claude
# → 테마 선택 → 로그인 → 폴더 trust 승인 후 종료
```

### 존재하지 않는 config 폴더 저장 시(생성+로그인 자동화)

프로필 추가/편집 폼에서 존재하지 않는 path를 저장하면 위 초기 설정을 자동화하는
확인 모달(`Create Config Folder`, `UiMode::ProfileDirConfirm`)이 뜬다:

- **Create**(기본 포커스): `create_dir_all`로 폴더 생성 → 프로필 저장 → 메인 루프가
  TUI를 해제하고 로그인용 에이전트를 실행(`resume::run_login`). 에이전트 종료 후
  TUI로 복귀하며 세션 재스캔 + 해당 프로필 사용량 증분 조회를 수행한다.
- **Cancel/esc**: 입력을 유지한 채 폼으로 복귀(경로 수정 가능).
- 로그인 실행은 resume/new 세션과 달리 **cd 없이 s7s의 현재 폴더**에서
  **플래그 없는 기본 명령**(`claude`/`codex`/`agy`)에 env 접두만 붙여 실행한다 —
  usage 조회가 s7s 실행 폴더에서 돌기 때문에, 로그인 중 이 폴더를 trust하면
  이후 usage 조회도 통과한다.
- **Antigravity 추가 경로**는 env 주입이 불가해 로그인 실행이 무의미하므로
  폴더 생성+저장만 수행하고 안내 메시지를 표시한다
  (`profile::login_runnable()` 판정).

- **trust는 폴더 단위**로 `<dir>/.claude.json`의 `projects.<path>.hasTrustDialogAccepted`에
  저장된다. 프로필 간 공유되지 않으므로 새 프로필에서는 폴더마다 처음 한 번씩 묻는다.
- usage 조회는 s7s 실행 폴더에서 claude를 띄우므로, **그 폴더를 trust**해야
  조회가 통과한다(미승인 시 `untrusted folder (trust prompt)` 실패).

## 세션/필터/사용량 연동

- 새 세션 생성 대화상자는 세션 조회/상세/프로필 화면 공용으로 `ctrl+n`으로 연다
  (프로필 화면의 기존 `enter` 열기는 `ctrl+n`으로 대체됨). 대화상자는 드롭다운
  컨트롤 3개와 하단 OK/Cancel 버튼 행(OK 왼쪽, Cancel 오른쪽)으로 구성된다:
  **Profile 콤보박스**(텍스트 입력 불가, 항목마다 이름 옆에 사용량 표시),
  **Model 콤보박스**(텍스트 입력 불가 — 항목·초기 선택·OK 비활성 규칙은
  [Model Selection](models.md) 참고), **Folder 콤보박스**(텍스트 입력 가능).
  `tab`/`shift+tab`으로 Profile → Model → Folder → OK → Cancel 순환 포커스를
  이동하며(버튼도 각각 독립된 tab 스톱, `←`/`→`로도 버튼 간 이동 가능).
  포커스된 버튼은 밝은 파란색 박스, 비포커스 버튼은 밝은 회색 박스 + "left"
  라벨과 같은 gray dim 글자로 표시된다.
  - 기본 프로필: 조회/상세 화면은 선택 세션의 프로필, 프로필 화면은 선택 프로필.
  - 폴더 초기값·초기 포커스: 조회/상세 화면은 선택 세션의 폴더가 채워진 채
    Profile에 포커스, 프로필 화면은 폴더가 비어 있고 Folder에 포커스한 채
    Folder 드롭다운을 열린 상태(첫 항목 하이라이트)로 시작한다.
  - `enter`는 포커스된 컨트롤의 기본 동작이다: 버튼은 해당 기능 수행(OK=세션
    시작, Cancel=취소), 드롭다운은 닫힘 상태면 목록 열기, 열림 상태면 커서
    항목을 확정(commit)한 뒤 닫기(커서가 목록에 없으면 입력창 텍스트를 그대로
    확정). 전역 `enter`=OK(세션 시작) 단축키는 제거됐다 — 세션 시작은 OK 버튼에
    포커스한 뒤 `enter`.
  - `→`(닫힌 드롭다운)도 목록을 연다. Profile/Model(편집 불가)은 현재 선택에
    커서를 두고 열고, Folder는 첫 항목에 커서를 둔다.
  - `↑`/`↓`는 드롭다운이 닫힌 상태면 컨트롤 간 세로 포커스 이동
    (Profile ↔ Model ↔ Folder ↔ Buttons, 양끝 정지·랩 없음), 열린 상태면 목록
    커서 이동이다. 열린 목록은 상하 순환한다 — 최상단에서 `↑`는 (닫지 않고) 맨 아래,
    최하단에서 `↓`는 맨 위 항목으로 이동한다.
  - 드롭다운 공통(열린 상태): `space`는 선택만 하고 열린 상태 유지(폴더는
    입력창에 즉시 반영), `esc`는 선택 없이 닫기. `tab`/`shift+tab`은 커서
    항목을 확정한 뒤 닫고 포커스를 이동한다(commit-then-move — 선택 없이 닫는
    경로는 `esc` 전용).
  - Folder 콤보: 타이핑하면 드롭다운이 자동으로 열리고(`enter`/`→`로도 열 수
    있음), 입력 텍스트에 매칭되는 폴더가 상단(보통색), 비매칭 폴더가 하단
    ("left" 라벨과 같은 soft dim 색)에 표시된다(숨기지 않음). 열린 상태의 `→`는
    텍스트 커서만 우이동한다(선택+닫기 complete 기능은 제거됨 — 커서 폴더를
    입력창에 반영하려면 `space`, 반영+닫기는 `enter`).
  - 세션 시작은 OK 버튼 `enter`(폴더가 빈 상태면 "Select a folder first" 오류를
    버튼 행 좌측에 표시), 취소는 Cancel 버튼/`esc`. 실행 후 세션 화면으로
    돌아오며 재스캔으로 새 세션 저장분을 반영한다.
- 스캔은 프로필 루프로 수행하며 각 세션에 `profile_id`를 부여한다(캐시 값은
  신뢰하지 않고 스캔 시 항상 재부여 — 프로필 삭제/재생성 시 stale 방지).
- rename(`rename.rs`)과 세션 삭제의 Antigravity 메타 정리가 쓰는 메타 경로는
  `session.profile_id → Profile.path`에서 파생한다(46차). 소속 프로필을 찾지
  못하면 기본 경로로 폴백하지 않는다 — rename은 오류 표시 후 중단, 삭제의 agy
  메타 정리는 스킵(본문 파일 삭제는 진행).
- 번호가 없는 프로필도 스캔한다(세션 목록은 전체 표시가 스펙). 헤더에는 최대 5개
  프로필만 `<1>`~`<5>`로 표시하며 세션 화면의 같은 번호 키로 필터링한다.
- 프로필 테이블의 1~3행은 기본 `Claude` → `Codex` → `Antigravity` 순서로 고정한다.
  사용자 추가 프로필은 4행부터 등록 순서대로 표시한다.
- 프로필 화면에서 `1`~`5`를 누르면 선택 프로필을 해당 번호 위치에 삽입한다.
  뒤 번호는 한 칸씩 밀리고, 5개가 이미 지정된 경우 기존 5번은 번호가 해제된다.
  `space`는 번호를 토글한다. 지정된 프로필은 번호를 해제하고 뒤 번호를 한 칸씩
  당기며, 번호가 없는 프로필은 현재 마지막 번호 다음에 추가한다. 5개가 모두
  지정된 상태에서 추가하려 하면 오류 대화상자를 표시하고 상태를 변경하지 않는다.
  번호 순서는 별도 `shortcut` 필드로 관리하며, 프로필 테이블 행은 번호 변경과
  관계없이 등록 순서로 고정한다.
- 프로필 삭제는 목록·필터·사용량 상태·세션 목록에서만 제거하고 실제 폴더는 유지한다.
- agent 필터(`a`)·folder 필터(`f`) 대화상자는 항목 체크(`space`) 즉시 선택을
  `filter`에 반영하고 `recompute()`를 호출해, 대화상자를 닫지 않아도 뒷쪽 세션
  목록이 실시간으로 갱신된다(`sync_agent_selection_to_filter` /
  `sync_folder_selection_to_filter`). Enter=확정 후 닫기, Esc=닫기(선택은 이미 반영됨).
- 사용량 조회 대상: path가 존재하는 프로필(Antigravity는 기본 경로만). 갱신 중에도
  직전 성공 값(`UsageEntry.last`)을 회색으로 유지 표시한다.
- 프로필 추가/편집 저장 시 세션은 전체 재스캔(mtime 캐시로 저렴)하되, 사용량은
  **저장된 프로필만 증분 조회**한다. 상세: [Usage Display](usage-display.md)의 "동작 방식" 절.

## 소스/대상 프로필 분리 (New Session with Context)

컨텍스트 첨부 새 세션([상세](session-context.md))에서 프로필의 역할은 둘로 나뉜다.

- **대상(target) 프로필**: New Session 대화상자에서 선택한 프로필. 실행되는
  에이전트의 계정·모델·env 주입(`CLAUDE_CONFIG_DIR`/`CODEX_HOME`)을 결정한다 —
  일반 새 세션과 완전히 동일.
- **소스(source) 프로필**: 참조되는 과거 세션의 소속 프로필. 대상 에이전트에
  **env로 주입되지 않으며**, 생성된 `s7s session ... --profile <소스ID>` 명령
  안에서만 이동한다. 새 세션 안에서 그 명령을 실행하는 자식 s7s 프로세스가
  전 프로필을 독립적으로 스캔해 올바른 소스를 해석한다.
- 소스 프로필이 삭제된 경우 OK 시점에 실행을 중단한다(다른 프로필 폴백 금지 —
  rename과 동일한 계정 안전 원칙). `s7s session` 해석도 요청 프로필 부재 시
  오류로 끝난다.
