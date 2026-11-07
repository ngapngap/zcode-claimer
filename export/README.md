# export output

File JSON sau claim sẽ nằm ở đây. Format đúng schema `provider_conns` của router:

```json
{
  "id": "<uuid v4>",
  "provider": "glm",
  "authType": "oauth",
  "name": "ZCode Daily Claim",
  "email": "user@example.com",
  "priority": 100,
  "isActive": true,
  "data": {
    "providerSpecificData": {
      "authMethod": "zcode-jwt",
      "username": "user@example.com",
      "userId": "<userId>",
      "zcodeJwtToken": "<JWT>",
      "zaiBusinessToken": "<token>"
    },
    "testStatus": "active"
  }
}
```

Import: `POST /api/providers` với body là JSON này (hoặc ghi trực tiếp vào `/home/ubuntu/.router/db/data.sqlite` bảng `provider_conns`).

`.gitkeep` giữ thư mục.
