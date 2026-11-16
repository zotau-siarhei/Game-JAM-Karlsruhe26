## ADDED Requirements

### Requirement: Пустая отображённая память буфера в WebKit
Страница браузерной версии SHALL подменять пустую или короткую память, которую WebKit отдаёт из
`getMappedRange` буфера, созданного с `mappedAtCreation`, своей памятью нужного размера и при
`unmap` записывать её в буфер через очередь устройства. Игра MUST NOT падать с «Length out of
range of buffer» при запуске в Safari.

#### Scenario: Safari отдал пустую память
- **WHEN** WebKit возвращает из `getMappedRange` ArrayBuffer короче запрошенного
- **THEN** игра загружается и рисует сцену как обычно

#### Scenario: Память отдана правильно
- **WHEN** `getMappedRange` возвращает память нужного размера
- **THEN** буфер заполняется как раньше, без лишней записи через очередь
