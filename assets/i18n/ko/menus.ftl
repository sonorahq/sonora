# track menu
menu-new-playlist = 새 플레이리스트
menu-import-playlist = 플레이리스트 가져오기…
menu-no-playlists = 플레이리스트 없음
menu-search-playlists = 플레이리스트 검색
menu-save-library = 라이브러리
menu-add-to-library = 즐겨찾기에 추가
menu-remove-from-library = 즐겨찾기에서 제거
menu-remove-tracks-from-playlist = { $count ->
    [one] 플레이리스트에서 { $count }트랙 제거
   *[other] 플레이리스트에서 { $count }트랙 제거
}
menu-remove-tracks-from-history = { $count ->
    [one] 재생 기록에서 { $count }트랙 제거
   *[other] 재생 기록에서 { $count }트랙 제거
}
menu-download = 다운로드
menu-remove-download = 다운로드 항목 제거
menu-downloading = 다운로드 중입니다. 중지하려면 클릭
menu-song-radio = 곡 라디오로 이동
menu-go-to-album = 앨범으로 이동
menu-go-to-artist = 아티스트로 이동
menu-go-to-playlist = 플레이리스트로 이동
menu-view-details = 상세 정보 보기
menu-copy-link = 링크 복사
menu-select-all = 전체 선택
menu-remove-from-queue = 재생 목록에서 제거
menu-export-playlist = 플레이리스트 내보내기…
menu-add-playlist-to-library = 라이브러리에 추가
menu-remove-playlist-from-library = 라이브러리에서 제거
menu-make-playlist-public = 공개로 전환
menu-make-playlist-private = 비공개로 전환
menu-play-artist = 아티스트 재생

# playlist editor
menu-copy-link-none = 공유할 링크가 없습니다.
menu-copy-link-local = 로컬 아티스트와 플레이리스트에는 링크가 없습니다.
menu-play-next = { $count ->
    [1] 바로 다음에 재생
    [one] { $count }트랙을 바로 다음에 재생
   *[other] { $count }트랙을 바로 다음에 재생
}
menu-add-to-queue = { $count ->
    [1] 재생 목록에 추가
    [one] 재생 목록에 { $count }트랙 추가
   *[other] 재생 목록에 { $count }트랙 추가
}
menu-play-last = { $count ->
    [1] 맨 마지막에 재생
    [one] { $count }트랙을 맨 마지막에 재생
   *[other] { $count }트랙을 맨 마지막에 재생
}
menu-delete-track-files = { $count ->
    [1] 트랙 파일 삭제
   *[other] 트랙 파일 { $count }개 삭제
}
menu-add-to-playlist = { $count ->
    [1] 플레이리스트에 추가
    [one] 플레이리스트에 { $count }트랙 추가
   *[other] 플레이리스트에 { $count }트랙 추가
}

# smart rules
smart-rules-create-title = 새 스마트 플레이리스트
smart-rules-edit-title = 스마트 규칙
smart-rules-loading = 서버에서 규칙을 불러오는 중…
smart-rules-saving = 저장 중…
smart-rules-failed-title = 규칙을 읽을 수 없습니다.
smart-rules-match-all = 모두 일치
smart-rules-match-any = 하나라도 일치
smart-rules-match-toggle = 다음 규칙 중 모두 일치할지 하나라도 일치할지 선택
smart-rules-of-following = (다음 조건)
smart-rules-add-condition = 조건 추가
smart-rules-add-group = 그룹 추가
smart-rules-remove-group = 이 그룹 제거
smart-rules-remove-condition = 이 규칙 제거
smart-rules-empty-group = 아직 규칙이 없습니다.
smart-rules-field = 필드
smart-rules-operator = 연산자
smart-rules-value-placeholder = 값
smart-rules-range-and = ~
smart-rules-true = 예
smart-rules-false = 아니요
smart-rules-unknown-note = 서버에서 보낸 그대로 유지됨
smart-rules-synced-title = 파일에서 동기화되는 플레이리스트입니다.
smart-rules-synced-message = 다음 스캔 시 여기에서 수정한 내용이 덮어쓰일 수 있습니다.
smart-rules-sort-label = 정렬 기준
smart-rules-sort-placeholder = title, dateloved, random…
smart-rules-order-none = 정렬 방향 없음
smart-rules-order-asc = 오름차순
smart-rules-order-desc = 내림차순
smart-rules-order-toggle = 정렬 방향
smart-rules-limit-label = 개수 제한
smart-rules-limit-placeholder = 일치하는 모든 항목
smart-rules-problem-empty-group = 그룹에 규칙이 없습니다.
smart-rules-problem-missing-field = 조건에 필드가 지정되지 않았습니다.
smart-rules-problem-range = 범위에는 최솟값과 최댓값이 모두 필요합니다.
smart-rules-problem-days = 일수에는 숫자를 입력해야 합니다.
smart-rules-problem-playlist = 플레이리스트 규칙에는 플레이리스트를 선택해야 합니다.
smart-rules-problem-truth = 이 규칙에는 참/거짓 값이 필요합니다.
smart-rules-problem-compared = 입력한 값이 규칙에 맞지 않습니다.
smart-rules-problem-boolean = 불리언 필드에는 참/거짓 값이 필요합니다.
smart-rules-problem-bad-order = 정렬 순서는 asc 또는 desc여야 합니다.

# playlist editor
playlist-name-placeholder = 플레이리스트 이름
playlist-create-title = 플레이리스트 만들기
playlist-create-shelf = 저장 위치
playlist-edit-title = 플레이리스트 편집
playlist-cover-choose = 사진 선택
playlist-cover-remove = 커버 제거
playlist-cover-unopenable = Sonora에서 이 파일을 열 수 없습니다.
playlist-cover-too-large = 이 사진은 40 MB보다 큽니다.
playlist-cover-not-a-picture = JPEG, PNG 또는 WebP 이미지 파일이 아닙니다.

# cover viewer
playlist-delete-title = 플레이리스트 삭제
playlist-again-title = 다시 추가하시겠습니까?
playlist-again-confirm = 이 트랙은 이미 “{ $name }”에 있습니다. 중복해서 추가하시겠습니까?
playlist-again-add = 다시 추가

# confirm

# cover viewer
cover-view = 커버 보기
cover-zoom-in = 확대
cover-zoom-out = 축소
cover-zoom-fit = 창에 맞추기

# confirm
confirm-remove-library-title = 라이브러리에서 제거
confirm-remove-playlist-title = 플레이리스트에서 제거
confirm-remove-history-title = 재생 기록에서 제거
confirm-remove-songs = { $count ->
    [one] 라이브러리에서 이 곡을 제거하시겠습니까?
   *[other] 라이브러리에서 { $count }곡을 제거하시겠습니까?
}
confirm-remove-playlist-songs = { $count ->
    [one] 플레이리스트에서 이 곡을 제거하시겠습니까?
   *[other] 플레이리스트에서 { $count }곡을 제거하시겠습니까?
}
confirm-remove-history-songs = { $count ->
    [one] 재생 기록에서 이 곡을 제거하시겠습니까?
   *[other] 재생 기록에서 { $count }곡을 제거하시겠습니까?
}
confirm-remove-albums = { $count ->
    [one] 라이브러리에서 이 앨범을 제거하시겠습니까?
   *[other] 라이브러리에서 앨범 { $count }개를 제거하시겠습니까?
}
confirm-remove-artists = { $count ->
    [one] 즐겨찾기에서 이 아티스트를 제거하시겠습니까?
   *[other] 즐겨찾기에서 아티스트 { $count }명을 제거하시겠습니까?
}
confirm-remove-playlists = { $count ->
    [one] 라이브러리에서 이 플레이리스트를 제거하시겠습니까?
   *[other] 라이브러리에서 플레이리스트 { $count }개를 제거하시겠습니까?
}
confirm-delete-track-files-title = 트랙 파일을 삭제하시겠습니까?
confirm-delete-track-files = 파일이 디스크에서 영구적으로 삭제됩니다. 이 작업은 되돌릴 수 없습니다.
confirm-uninstall-widevine-title = Widevine 모듈을 제거하시겠습니까?
confirm-uninstall-widevine = Sonora가 Google에서 다운로드한 복사본을 삭제합니다. 다시 다운로드할 때까지 Apple Music 트랙을 재생할 수 없습니다.
confirm-remove-soloist-title = Soloist를 제거하시겠습니까?
confirm-remove-soloist = Sonora가 Soloist 프로그램, API 키 및 로그인 정보를 삭제합니다. Spotify는 다시 내장 플레이어를 사용하게 됩니다.
confirm-remove-downloads-title = 모든 다운로드 항목을 제거하시겠습니까?
confirm-remove-downloads = 모든 서비스에서 다운로드한 모든 곡이 이 컴퓨터에서 삭제됩니다. 다시 다운로드하기 전까지는 스트리밍으로 재생됩니다.
confirm-delete-named = “{ $name }”을(를) 삭제하시겠습니까? 이 작업은 되돌릴 수 없습니다.
