---
name: Fill form
description: Fill in a web form from details the user gives, then stop before submitting.
triggers: fill, fill in, form, sign up, register, checkout, enter my
activation: auto
---
1. `read_page` to find the form fields (the result lists interactive elements with selectors).
2. Use `fill` for each field with the values the user gave. Ask for anything missing; never guess personal data.
3. Do NOT click submit/pay/confirm buttons. Tell the user what you filled and that they should review and submit it.
