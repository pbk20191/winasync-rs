# comio COM/WinRT 노출 현황

*기준일: 2026-11-09*

## 방향 (2026-11-09 결정)

**최종 목표는 WinRT 개념으로 스크립트 월드까지 노출하는 것**이고, 타깃 스크립트 호스트는 **모던 쪽(Python / Node / .NET 스크립팅)만**이다. 활성화는 당분간 HKLM(개발 중 승격 허용), 배포 모델은 나중에 정한다.

이 결정이 정하는 것:

- **`.winmd`가 메인 산출물이다.** 옵션이 아니다.
- **`.tlb` + IDispatch는 동결된 호환 레이어다.** 지우지 않되 투자하지 않는다. tlbimp으로 뚫은 PowerShell 경로도 여기 묶인다.
- **클래식 스크립트 호스트는 범위 밖이다.** VBScript/WSH, VBA, Office 매크로는 WinRT보다 먼저 만들어졌고 동결돼 IDispatch만 안다. "레거시 COM을 버린다"와 "그 호스트들을 버린다"는 같은 문장이다.
- **폐기된 작업 둘**: 오퍼레이션 객체(상관 토큰), tlbimp interop 통합 테스트. 아래 "폐기" 절 참조.

수용 기준: **Python 스크립트가 리스너를 만들고 `AcceptAsync`를 await 한다.**

## 현재 상태

comio의 객체는 **WinRT ABI로 구현된 클래식 COM 컴포넌트**다. 아래 네 겹이 모두 동작하며, 각각 다른 청중을 위한 것이다.

| 겹 | 무엇을 하나 | 모듈 |
| --- | --- | --- |
| 활성화 | CLSID/ProgID로 `CoCreateInstance`. `DllRegisterServer`가 HKCU에 per-user 등록 (승격 불필요) | `activation.rs` |
| IDispatch | 스크립트용 늦은 바인딩. 블로킹 모양 — `Connect` / `Accept(ms)` / `WriteText` / `ReadText` | `dispatch.rs` |
| 타입 라이브러리 | dispatch 테이블에서 `.tlb`를 **생성**. 이른 바인딩 + coclass의 `[default, source]` | `typelib.rs` |
| 커넥션 포인트 | `IConnectionPointContainer` + `IProvideClassInfo2`. 싱크에 `ConnectionReceived` 전달 | `connpt.rs` |

`.tlb`를 IDL이 아니라 기존 테이블에서 생성하므로, 이른 바인딩과 늦은 바인딩이 멤버를 다르게 기술하는 일이 **구조적으로 불가능**하고 빌드에 `midl.exe`가 필요 없다.

빠져 있는 것은 WinRT 쪽 윗겹이다: `.winmd` 메타데이터가 없고, 활성화도 `RoActivateInstance`가 아니라 `CoCreateInstance`다. 그래서 vtable의 `IAsyncOperation` 비동기 모양은 Rust 외의 언어에서 읽을 방법이 없다.

## 호스트별 도달 범위

스크립트 호스트는 이벤트까지 전부 도달하고, 비동기 vtable은 Rust만 읽는다.

| 호스트 | 객체 생성 | 메서드 호출 | 이벤트 | `IAsyncOperation` |
| --- | --- | --- | --- | --- |
| WSH / VBScript | 검증 | 검증 | **검증** (`WScript.ConnectObject`) | 불가 |
| PowerShell 7 | 검증 | 검증 | **검증** — tlbimp interop 필요 | 불가 |
| C# / .NET | 됨 (tlbimp) | 됨 | 됨 — 같은 interop 경로 | 불가 (`.winmd` 없음) |
| C++ | 됨 (`#import` / vtable) | 됨 | 됨 | 수동 선언하면 |
| VBA | 됨 | 됨 | 같은 탐색 경로 (미검증) | 불가 |
| Rust | 직접 | 직접 | 직접 | **직접** |

"검증"은 실제로 돌리고 출력을 확인한 것이다. VBA는 WSH와 **동일한 탐색 경로**(class info → default source → `FindConnectionPoint`)를 쓰지만 직접 돌려보지는 않았다.

이 표는 **현재 도달 범위이지 목표가 아니다.** 위 "방향"대로 위쪽 세 줄(WSH/VBScript, VBA, 그리고 `.tlb` 경유 PowerShell)은 동결된 레이어에 속한다.

### PowerShell 레시피

`Register-ObjectEvent`는 `Type.GetEvent`로 이름을 찾는데 `System.__ComObject`는 답할 게 없다. interop 어셈블리가 그걸 메운다.

```text
regsvr32 /s rustwinui3loop.dll
TlbImp.exe "%LOCALAPPDATA%\rustwinui3loop\rustwinui3loop.tlb" ^
    /out:Rustwinui3loop.Interop.dll /namespace:Rustwinui3loop
```

```powershell
Add-Type -Path .\Rustwinui3loop.Interop.dll
# -ComObject 가 아니라 interop coclass 여야 한다
$l = New-Object Rustwinui3loop.NamedPipeConnectionListenerClass
Register-ObjectEvent -InputObject $l -EventName ConnectionReceived -SourceIdentifier conn -Action {
    $EventArgs.ReadText(64)
}
$l.Bind($name)
```

주의 두 가지.

- `ComEventsHelper.Combine`으로 내려가면 advise는 성공하지만 **싱크가 runspace 없는 스레드에서 scriptblock을 불러 죽는다**. `Register-ObjectEvent`만 동작한다.
- 전달은 **협조적**이다. PowerShell이 이벤트를 자기 큐에 넣고 파이프라인 스레드의 문장 경계에서 실행하므로, 메인이 긴 호출 하나에 막히면 제 핸들러도 지연된다. 그럴 땐 `Start-ThreadJob` 안의 블로킹 `Accept`가 맞다.

tlbimp은 .NET Framework SDK 도구다. **빌드 단계에 넣지 않는다** — midl을 피한 이유를 그대로 되살리게 된다. 소비자 쪽 선택적 단계로 둔다.

## 측정으로 밝혀진 것

세 건 모두 **in-process 테스트는 전부 통과하는데 외부 호스트에서만 깨지는** 종류였다.

### `CO_E_NOTINITIALIZED` — 싱크 호출이 조용히 실패

스레드풀 스레드는 `CoInitializeEx`가 불린 적이 없다. 이런 스레드는 **암묵적 MTA**가 서비스하는데, 그건 MTA가 존재할 때만이다. `cscript`처럼 호스트 스레드가 전부 STA면 프로세스에 MTA가 없어 첫 `Invoke`가 실패하고 이벤트가 사라진다.

해법은 첫 `Advise` 때의 `CoIncrementMTAUsage` (`connpt.rs`). 이건 **스레드를 초기화하지 않고** 프로세스에 MTA가 살아있게만 한다. 스레드풀 스레드에 `CoInitializeEx`를 부르면 안 되는 이유:

- 콜백 끝에 `CoUninitialize`했다가 하필 마지막 MTA 참조였다면 **프로세스 전체의 마샬링된 프록시가 죽는다**.
- 해제하지 않으면 남의 콜백을 실행할 공유 스레드의 아파트먼트 상태를 영구히 바꿔놓는 것이다.

> MS 문서의 "WinRT를 쓰면 `CoInitializeEx` 대신 `Windows::Foundation::Initialize`"는 **내가 소유한 스레드를 초기화할 때**의 이야기다. 우리 콜백 스레드는 WinRT 클래스를 **활성화하지 않고** 이미 존재하는 인터페이스 포인터로 메서드만 부르므로, 아파트먼트만 필요하고 WinRT 초기화는 불필요하다. (`listener.rs:42` 참조 — 객체는 Rust 팩토리에서 오지 `RoActivateInstance`가 아니다.)

### `TYPE_E_LIBNOTREGISTERED` — 타입 라이브러리 LCID

`CreateTypeLib2`가 라이브러리 LCID를 **사용자 기본값**(여기선 412, ko-KR)으로 잡아 그 아래 등록했고, `LoadRegTypeLib`는 중립 0으로 찾아 못 찾았다. `SetLcid(0)`으로 고정. 언어 설정이 다른 장비에서만 재현되었을 버그다. `unregister`는 이제 `TypeLib\{libid}` 서브트리 전체를 지워 낡은 LCID 등록이 살아남지 못하게 한다.

### PowerShell 이벤트 — 틀리게 문서화했던 것

"PowerShell은 이 이벤트를 받을 수 없다"고 적었는데, 그건 **타입 라이브러리를 만들기 전 기준**이었다. `.tlb`를 추가한 순간 tlbimp이 필요한 글루(`DListenerEvents_EventProvider`, 싱크 헬퍼, 각 coclass의 .NET `ConnectionReceived` 이벤트)를 전부 생성할 수 있게 됐는데, 그걸 다시 확인하지 않았다. 문서는 수정했다.

## 막힌 것: `IInspectable`이 비어 있다

우리 객체는 진짜로 `IInspectable`이다 — `IClosable`, `IInputStream`, `IOutputStream`이 전부 WinRT 인터페이스라 QI가 성공한다. 그런데 세 메서드가 전부 `S_OK`로 **빈 값**을 돌려준다. "모른다"가 아니라 "없다"고 자신 있게 답하는 것이라 더 나쁘다.

| 메서드 | 실제 응답 | 어디서 결정되나 | 훅 |
| --- | --- | --- | --- |
| `GetIids` | `count=0`, `values=null` | `IInspectable_Vtbl::new`가 설치하는 **자유 함수**에 하드코딩 | **없음** — 타입 파라미터를 아예 안 받음 |
| `GetRuntimeClassName` | `""` | `#[implement]` 목록의 **첫 인터페이스**의 `RuntimeName::NAME` | 없음 — `#[interface]`가 빈 impl을 자동 생성해 덮어쓰면 충돌 |
| `GetTrustLevel` | `0` (BaseTrust) | 매크로가 직접 넣음 | 없음 — 속성 파서가 인터페이스 경로만 읽음 |

근본 원인은 identity vtable이 생성된 `_Impl` 타입의 **연관 상수**고, `QueryInterface`가 `IID_IInspectable`을 가장 먼저 거기로 보낸다는 것이다. 끼어들 틈이 없다. `windows-implement` 0.100도 구조가 같아 업그레이드로 풀릴 문제가 아니다.

### 반쪽 수정을 안 한 이유

`GetRuntimeClassName`만은 편법이 있다 — `RuntimeName`을 손으로 단 마커 인터페이스를 `#[implement]` 첫 줄에 넣으면 된다. 안 했다.

**런타임 클래스 이름은 "이 이름이 WinRT 메타데이터로 해석된다"는 약속이다.** `.winmd`를 안 내놓으니 `RoGetActivationFactory`는 여전히 실패한다. 이름을 채우면 객체의 주장이 **더 그럴듯해질 뿐 참이 되지는 않는다**. 빈 문자열이 차라리 "해석될 것처럼 생긴 거짓말"은 아니다. 그리고 `GetIids`는 어차피 못 고친다.

즉 이 둘을 정직하게 만드는 일은 **`.winmd` 작업의 일부**이지 그보다 먼저 할 수 있는 단계가 아니다.

다만 `.winmd`를 내놓는 것만으로 저절로 고쳐지지도 않는다 — `#[implement]`가 막고 있는 건 그대로다. 위 "비용 3"이 그 이야기이고, 다음 작업 1번이 그걸 확인한다.

### 대신 한 것

`comio::tests::iinspectable_is_shaped_but_hollow` — characterisation 테스트로 현재 응답을 못 박았다. **컨트롤**로 진짜 WinRT 클래스(`Windows.Storage.Streams.Buffer`)를 같은 코드 경로로 통과시켜 `GetIids`가 0이 아니고 이름도 비어 있지 않음을 확인한다. 이게 없으면 "빈 응답"이 호출 방식 탓일 수도 있다. `windows-rs`를 올렸을 때 이게 바뀌면 조용히 지나가지 않고 테스트가 깨진다.

## `.winmd`로 가는 길

방향은 정해졌으므로 아래는 비교가 아니라 **무엇이 걸려 있는지**다.

### 얻는 것

비동기 모델을 **번역하지 않고 그대로 기술**한다. `IAsyncOperation<T>`가 네이티브로 나오고, `Completed`가 콜백이고, `Cancel`과 `ErrorCode`가 딸려온다. 커넥션 포인트도 블로킹 투영도 필요 없다. 컴파일된 소비자에게 `.tlb`가 **번역**인 데 반해 `.winmd`는 **기술**이다.

소비 경로는 프로젝션 생성이다. [PyWinRT](https://github.com/pywinrt/pywinrt)는 `pywinrt.exe`로 우리 `.winmd`에서 Python 프로젝션을 생성하고, 생성된 패키지의 `__init__.py` 옆에 우리 DLL을 둔다. [CsWinRT](https://github.com/microsoft/CsWinRT/blob/master/docs/authoring.md)도 같은 모양이다.

### 비용 1 — 활성화가 HKLM으로 간다

[비패키지 서드파티 WinRT](https://blogs.windows.com/windowsdeveloper/2019/04/30/enhancing-non-packaged-desktop-apps-using-windows-runtime-components/)의 활성화는 `HKLM\SOFTWARE\Microsoft\WindowsRuntime\ActivatableClassId`(머신 전역, 승격 필요) 아니면 **소비하는 앱 쪽** `application.manifest`의 `<activatableClass>`(reg-free WinRT)다. 후자는 `python.exe` / `node.exe` 같은 공유 인터프리터엔 못 넣는다. HKCU 경로는 패키지 앱용으로 보인다.

지금의 "승격 없는 `regsvr32` 한 줄"을 잃는다. 개발 중엔 승격을 받기로 했고, **배포 모델은 미정**이다.

### 비용 2 — 표면의 기술을 어디에 둘 것인가 (미결)

순수 Rust로 가능하다 — `windows-metadata`는 ECMA-335를 읽고 **쓰며**, `windows-rdl`이 RDL → `.winmd` 컴파일을 한다. midl/cl.exe 불필요. 그러니 진짜 작업량은 라이터가 아니다.

우리 dispatch 테이블은 **블로킹 dispinterface 모양**을 기술하는데, `.winmd`가 기술해야 하는 건 **비동기 vtable 모양**(`IConnectionListener`, `IAsyncOperation` 반환)이고 그건 Rust `#[interface]` 선언에만 존재한다. 테이블에서 자동 생성이 안 되고, 그 표면에 대한 **새 기술**이 필요하며, `.tlb`에서 구조적으로 막아둔 드리프트 위험이 거기서 다시 들어온다.

선택지: RDL 파일을 소스 오브 트루스로 두거나, `windows-metadata` 라이터로 테이블에서 직접 방출하거나.

### 비용 3 — `IInspectable`이 블로커일 수 있다 (미확인)

제대로 된 WinRT 런타임 클래스는 `GetRuntimeClassName`으로 자기 이름을 돌려줘야 하는데, 위 "막힌 것" 절대로 `windows-rs`의 `#[implement]`로는 **불가능하다**. `.winmd`를 내놓아도 이건 그대로 남는다.

프로젝션이 실제로 그걸 부르는지가 관건이다. 정적으로 프로젝션된 호출 경로는 안 부를 수도 있지만, 박싱/언박싱이나 `object` → 구체 타입 해석에서는 쓰인다. **부른다면 `windows-rs`의 identity vtable 문제를 먼저 풀어야 한다**(`windows-implement` 포크, 또는 identity를 손으로 작성). 이게 이 방향의 유일한 미확인 블로커다.

## 다음 작업

`.winmd` 트랙이 주 경로다. 1번이 블로커 확인이라 먼저 간다 — 결과에 따라 2번의 크기가 달라진다.

1. **프로젝션이 `GetRuntimeClassName`을 부르는가** · 작음 · **블로커 확인**
   위 "비용 3". `windows-rs`의 `#[implement]`로는 정직한 답을 못 내놓으므로, 프로젝션이 이걸 요구하면 identity vtable 문제를 먼저 풀어야 한다. 손으로 만든 최소 `.winmd` 하나 + PyWinRT 또는 CsWinRT로 확인하는 게 가장 빠르다.

2. **`.winmd` 생성 + `DllGetActivationFactory` + HKLM 등록** · 큼
   먼저 "비용 2"의 선택(RDL vs 직접 방출)을 정한다. 1번 결과에 따라 identity vtable 작업이 여기 붙는다.

3. **Python end-to-end 검증** · 중간 · **수용 테스트**
   PyWinRT로 프로젝션을 생성하고, Python 스크립트가 리스너를 만들어 `AcceptAsync`를 await 한다. 이게 되면 목표 달성이다.

4. **hvsock TLS** · 중간 · *독립 트랙*
   메모리에 "진짜 gap"으로 남은 유일한 항목(2026-11-08 보류). WinRT의 TLS는 `StreamSocket`의 `SocketProtectionLevel`에만 있고, hvsock은 **host↔guest 신뢰 경계**를 넘으므로 여기서만 실제 필요하다(로컬 파이프엔 무의미). Schannel을 `OverlappedDuplexStream` 위에 얹으면 `IInputStream`/`IOutputStream` 모양이 유지된다. `.winmd` 트랙과 겹치지 않으니 언제 해도 된다.

5. **out-of-process 활성화** · 중간 · *나중*
   WinRT에도 `OutOfProcessServer` 모델이 있다. 마샬링·아파트먼트·수명을 전부 강제로 통과시키는 값은 그대로지만, in-proc으로 3번을 먼저 끝내는 편이 낫다.

### 폐기 (2026-11-09 방향 결정에 따라)

- **오퍼레이션 객체(상관 토큰)** — IDispatch에서 비동기를 흉내 내려던 물건이다. WinRT 소비자는 `IAsyncOperation`을 그대로 받으므로 만들 이유가 사라졌다.
- **tlbimp interop 통합 테스트** — 동결된 레이어를 위해 .NET Framework SDK 의존을 테스트에 들이는 건 잘못된 투자다. 레시피는 위 "PowerShell 레시피"에 남겨 둔다.
- **`IAsyncOperation` 교차 아파트먼트 마샬링 측정** — `.winmd` 판단 근거로 필요했던 것인데 판단이 끝났다. 메타데이터를 내놓으면 자연히 답이 나온다.

### 가치 낮음

- **`DllCanUnloadNow` 객체 카운팅** — 지금 무조건 `S_FALSE`. 이유는 `activation.rs` 문서에 있고, 고치면 `CoFreeUnusedLibraries`가 동작. 위험 대비 이득 작음.
- **`EnumConnectionPoints` / `EnumConnections`** — 디자인타임 브라우저 전용. **권장하지 않음**, 소비자가 없다. 동결 레이어이기도 하다.

### 위생

- 테스트 2곳의 `CoInitializeEx` → `RoInitialize`. 깨끗한 프로세스 3개로 재본 결과 **동작 차이는 없다**(초기화 없음 / `CoInitializeEx(MTA)` / `RoInitialize(MULTITHREADED)` 모두 WinRT 활성화·vtable 호출·`GetRuntimeClassName`·async op 동일). 버그 수정이 아니라 의도 표현이다. `Win32_System_WinRT` 피처가 필요하다.
- 기존 clippy 경고 24건 (`tp` / `rio` / `main.rs` / `listener.rs`). 이번 작업과 무관한 기존 것들.

## 커밋 상태

| 커밋 | 내용 |
| --- | --- |
| `ace1f38` | 타입 라이브러리와 커넥션 포인트 |
| `c483249` | IDispatch |
| `abdb902` | 활성화 |
| `aa05a16` | 클라이언트 타입을 COM 커넥터로 |

**미커밋** (워킹 트리): `src/comio/activation.rs`, `src/comio/connpt.rs`, `src/comio/mod.rs` — 문서 교정 + `iinspectable_is_shaped_but_hollow` 테스트.

### 재현

```powershell
# 등록 / 해제 (HKCU, 승격 불필요)
Start-Process regsvr32 -ArgumentList '/s', 'target\debug\rustwinui3loop.dll' -Wait
Start-Process regsvr32 -ArgumentList '/s', '/u', 'target\debug\rustwinui3loop.dll' -Wait

# 전체 검증
cargo test --all-features
cargo clippy --all-targets --all-features
```

타입 라이브러리는 `DllRegisterServer`가 `%LOCALAPPDATA%\rustwinui3loop\rustwinui3loop.tlb`에 쓴다.
