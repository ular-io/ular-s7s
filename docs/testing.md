# Testing Guide

이 프로젝트의 rename/session-title 관련 로직은 외부 agent CLI의 내부 저장 구조에 의존한다.
따라서 테스트는 "단위 테스트 통과"만으로 끝나면 안 된다.

## Required checks

코드 변경 시 최소한 아래를 수행한다.

1. `cargo fmt --all`
2. `cargo test -q`
3. rename/session-title 관련 코드를 바꿨다면 실제 로컬 CLI 수동 검증
4. 패널 포커스/스타일을 바꿨다면 실제 TUI 수동 검증

## Why unit tests are not enough

- 외부 CLI는 업그레이드 시 저장 파일 구조를 바꿀 수 있다.
- 같은 옵션 이름이라도 동작이 달라질 수 있다.
- exit code는 성공이어도 실제 제목 이벤트를 쓰지 않을 수 있다.
- 특정 agent는 비대화형 환경과 대화형 환경의 rename 동작이 다를 수 있다.

## Unit test policy

rename/session-title 코드에 대한 테스트는 다음을 커버해야 한다.

- 명시적 rename이 자동 제목보다 우선하는지
- 캐시 재적용 시 ID 기준으로 제목이 덮어써지는지
- agent별 메타 파일 경로가 올바른지
- CLI rename 성공 시 폴백 쓰기가 중복되지 않는지
- CLI rename 실패 시 저장소 직접 갱신으로 폴백하는지

## Current automated coverage

- Claude 메타 JSON 갱신
- Claude JSONL 제목 이벤트 append
- Claude CLI rename 성공 시 중복 append 방지
- Codex `session_index.jsonl` 갱신
- Codex sqlite `threads.title` 갱신
- Antigravity `annotations/*.pbtxt` 갱신
- 메타 경로의 프로필 루트 파생(세 agent 공통 — rename 테스트가 임의 루트 사용)
- 소속 프로필 미발견 시 rename 중단(기본 경로 폴백 금지)

## Manual verification checklist

rename/session-title 로직을 바꿨거나 외부 CLI가 업그레이드됐으면 아래를 직접 확인한다.

1. 임시 세션 생성
2. 제목 변경 실행
3. TUI 목록 제목 변경 확인
4. agent 원본 CLI에서 같은 세션 재열기
5. 세션 제목 유지 확인
6. 앱 재시작 후 목록 재스캔
7. `--rebuild-cache` 후에도 제목 유지 확인

## Agent-specific manual checks

### Claude

- `claude --resume <id> --name <title> -p --output-format json` 실행
- JSONL에 `custom-title` / `agent-name` 이벤트가 생겼는지 확인
- `~/.claude/sessions/*.json`의 `name`, `nameSource` 확인
- `/rename ...` 프롬프트가 여전히 비대화형에서 막히는지 확인

### Codex

- `~/.codex/session_index.jsonl`의 `thread_name` 확인
- `~/.codex/state_*.sqlite`의 `threads.title` 확인
- 비대화형 `codex exec resume <id> "/rename ..."` 동작 변화 여부 확인

### Antigravity

- `annotations/<id>.pbtxt`의 `title:"..."`
- `conversation_metadata.json`의 `summary.Title`
- `agy --print "/rename ..."`가 실제 rename 흔적을 남기는지 재검증
- `--conversation <id>`가 실제 대상 세션을 사용하는지 재검증

## Session context / contextual launch checks

세션 컨텍스트(`src/session_context/`·`s7s session`)나 New Session with Context
경로를 바꿨거나 agent CLI가 업그레이드됐으면 아래를 확인한다
([상세](./session-context.md)).

1. `cargo test real_data_turn_parity -- --ignored --nocapture` — 실데이터 전수
   턴 패리티(목록 Q 수 == 컨텍스트 턴 수, claude/codex 엄격).
2. `s7s session <실제 ID>` — reference 출력에 중지/대기/언어 지시가 없는지,
   `--turn`/`--user-only`/`--bootstrap` 각 프로젝션 확인.
3. 실제 contextual 런치(각 agent): 부트스트랩 프롬프트가 transcript에 user 턴으로
   기록되는지, `s7s session ... --bootstrap`이 성공하는지, 과거 작업/파일 변경이
   없는지, 레디 메시지가 소스 유저 턴의 주 언어인지.
4. 런치된 세션이 s7s 목록에서 Q 수·프리뷰·제목·검색을 오염시키지 않는지
   (부트스트랩만 있는 세션은 목록에 비노출), `--rebuild-cache` 후에도 동일한지.
5. 초기 프롬프트 주입 방식은 CLI 업그레이드 시 재검증: claude/codex는 positional
   (`[prompt]`/`[PROMPT]`), agy는 `--prompt-interactive`(positional 미지원).

## Keyboard protocol checks

- kitty 프로토콜 지원 터미널에서 `ctrl+shift+n` → contextual, `ctrl+n` → ordinary
  분리 동작 확인.
- 레거시 터미널·tmux에서 `:` 팔레트 폴백 확인.
- 종료·에이전트 핸드오버 후 keyboard enhancement가 남지 않는지 확인
  (핸드오버된 CLI의 키 입력이 정상인지).

## When to update tests and docs

아래 중 하나가 바뀌면 테스트와 문서를 같이 갱신한다.

- CLI 옵션
- 세션 저장 경로
- 제목 필드 이름
- 세션 ID 추출 방식
- 캐시 구조
- rename 성공 판정 방식

## Related docs

- [Panel Focus Style](./panel-focus-style.md)
- [Session Title Compatibility](./session-title-compat.md)
- [Session Context](./session-context.md)

